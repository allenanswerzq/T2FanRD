//! Fan daemon for T2 Macs. Each curve watches the hottest of its sensors;
//! the fans follow the hottest curve, speed up at once and slow down
//! gently. See README.md and t2fanrd.toml.

mod config;
mod control;
mod curve;
mod fan;
mod sensor;

use std::{
    fs::File,
    io::Write,
    os::fd::AsRawFd,
    path::PathBuf,
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use clap::Parser;
use signal_hook::consts::{SIGINT, SIGTERM};

use crate::{config::Config, control::Control, curve::Curve, fan::Fans};

const PID_FILE: &str = "/run/t2fand.pid";
const TRACE_EVERY: Duration = Duration::from_secs(1);

#[derive(Parser)]
#[command(version, about = "Fan daemon for T2 Macs")]
struct Cli {
    /// Config file.
    #[arg(long, default_value = "/etc/t2fanrd.toml")]
    config: PathBuf,
    /// Compute and print fan speeds without touching the fans.
    #[arg(long)]
    dry_run: bool,
    /// Print temperatures and fan speeds every second.
    #[arg(long)]
    trace: bool,
    /// Print every sensor a curve can use, then exit.
    #[arg(long)]
    list_sensors: bool,
    /// Hold every fan at full speed instead of following the curves.
    #[arg(long)]
    full_speed: bool,
}

fn main() -> ExitCode {
    match run(&Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    if cli.list_sensors {
        for (name, temp) in sensor::list()? {
            let temp = temp.map_or_else(|| "?".to_owned(), |t| format!("{t:.1} C"));
            println!("{name:<24} {temp:>8}");
        }
        return Ok(());
    }

    let config = Config::load(&cli.config)?;
    let mut curves = config
        .curves
        .iter()
        .map(Curve::open)
        .collect::<Result<Vec<_>>>()?;
    let mut control = Control::new(config.output);

    let _lock = if cli.dry_run { None } else { Some(lock()?) };
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [SIGINT, SIGTERM] {
        signal_hook::flag::register(signal, stop.clone()).context("cannot handle signals")?;
    }

    let mut fans = Fans::open(&sensor::smc_dir()?, !cli.dry_run)?;
    let interval = Duration::from_secs_f64(config.interval);
    let stale_after = Duration::from_secs_f64(config.stale_after);
    let mut sleep_clock = SleepClock::new();
    let mut failure: Option<String> = None;
    let mut traced: Option<Instant> = None;

    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if let Some(slept) = sleep_clock.slept() {
            println!(
                "Woke up after {}s of sleep; taking the fans back",
                slept.as_secs()
            );
            curves.iter_mut().for_each(Curve::reset);
            control.reset();
            fans.reclaim()?;
        }

        let mut demand = 0.0_f64;
        let mut failed = None;
        for curve in &mut curves {
            match curve.update(now, stale_after) {
                Ok(percent) => demand = demand.max(percent),
                Err(err) => failed = failed.or(Some(err)),
            }
        }
        let percent = if cli.full_speed {
            100.0
        } else if let Some(err) = failed {
            if failure.as_ref() != Some(&err) {
                eprintln!("Sensor failed, fans at full speed: {err}");
            }
            failure = Some(err);
            control.hold(now, 100.0);
            100.0
        } else {
            if failure.take().is_some() {
                println!("Sensors work again");
            }
            control.update(now, demand)
        };
        fans.set(percent, now)?;

        if (cli.trace || cli.dry_run) && traced.is_none_or(|t| now.duration_since(t) >= TRACE_EVERY)
        {
            traced = Some(now);
            trace(&curves, demand, percent, &fans);
        }
        std::thread::sleep(interval.saturating_sub(now.elapsed()));
    }
    if !cli.dry_run {
        println!("Stopping; the SMC controls the fans again");
    }
    Ok(())
}

fn trace(curves: &[Curve], demand: f64, percent: f64, fans: &Fans) {
    let mut line = String::new();
    for c in curves {
        line += &format!("{} {:.1}C {:3.0}%  ", c.name(), c.temp, c.percent);
    }
    line += &format!("| want {demand:3.0}% set {percent:5.1}% |");
    for f in &fans.fans {
        let rpm = f.rpm().map_or_else(|| "?".to_owned(), |r| r.to_string());
        line += &format!(" {} {rpm}/{}", f.name, f.target);
    }
    println!("{line}");
}

/// Holds the PID file locked so only one daemon drives the fans.
fn lock() -> Result<File> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        bail!("t2fanrd must run as root (or use --dry-run)");
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(PID_FILE)
        .with_context(|| format!("cannot open {PID_FILE}"))?;
    // SAFETY: the descriptor stays open for the call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("t2fanrd is already running");
    }
    file.set_len(0)?;
    write!(file, "{}", std::process::id())?;
    Ok(file)
}

/// Notices system sleep: the boot clock counts it and the monotonic clock
/// doesn't, so their difference grows by the time slept.
struct SleepClock {
    gap: Duration,
}

impl SleepClock {
    fn new() -> Self {
        Self { gap: Self::gap() }
    }

    fn gap() -> Duration {
        let read = |clock| {
            let mut ts = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            // SAFETY: ts is a valid timespec for the call to fill in.
            unsafe { libc::clock_gettime(clock, &raw mut ts) };
            Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
        };
        read(libc::CLOCK_BOOTTIME).saturating_sub(read(libc::CLOCK_MONOTONIC))
    }

    fn slept(&mut self) -> Option<Duration> {
        let gap = Self::gap();
        let slept = gap.saturating_sub(self.gap);
        self.gap = gap;
        (slept >= Duration::from_secs(1)).then_some(slept)
    }
}
