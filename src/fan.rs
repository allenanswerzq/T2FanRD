//! The T2's fans. The SMC takes a target RPM in fanN_output while fanN_manual
//! is 1, and takes the fans back after sleep, so the daemon checks that it
//! still has them and reclaims them.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{ensure, Context, Result};

const CHECK_EVERY: Duration = Duration::from_secs(5);
const STALL_AFTER: Duration = Duration::from_secs(10);

pub struct Fan {
    pub name: String,
    base: PathBuf,
    min: u32,
    max: u32,
    pub target: u32,
    written: Option<u32>,
    slow_since: Option<Instant>,
    stall_reported: bool,
}

impl Fan {
    fn path(&self, suffix: &str) -> PathBuf {
        self.base.with_file_name(format!("{}{suffix}", self.name))
    }

    fn read(&self, suffix: &str) -> Result<u32> {
        let path = self.path(suffix);
        std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?
            .trim()
            .parse()
            .with_context(|| format!("{} is not a number", path.display()))
    }

    fn write(&self, suffix: &str, value: u32) -> Result<()> {
        let path = self.path(suffix);
        std::fs::write(&path, value.to_string())
            .with_context(|| format!("cannot write {}", path.display()))
    }

    /// The speed the fan reports.
    pub fn rpm(&self) -> Option<u32> {
        self.read("_input").ok()
    }

    fn rpm_for(&self, percent: f64) -> u32 {
        self.min + (f64::from(self.max - self.min) * percent / 100.0).round() as u32
    }

    /// Takes the fan back if the SMC or anyone else changed it, and warns
    /// once about a fan that doesn't turn.
    fn check(&mut self, now: Instant) -> Result<()> {
        if self.read("_manual")? != 1 {
            eprintln!("{}: the SMC has the fan, taking it back", self.name);
            self.write("_manual", 1)?;
            self.written = None;
        } else if self.written.is_some() && self.read("_output").ok() != self.written {
            eprintln!(
                "{}: target speed changed elsewhere, setting it again",
                self.name
            );
            self.written = None;
        }

        let rpm = self.rpm().unwrap_or(0);
        if rpm >= self.min / 2 {
            self.slow_since = None;
            self.stall_reported = false;
        } else if !self.stall_reported
            && now.duration_since(*self.slow_since.get_or_insert(now)) >= STALL_AFTER
        {
            eprintln!(
                "{}: only {rpm} RPM for {}s; is it stuck?",
                self.name,
                STALL_AFTER.as_secs()
            );
            self.stall_reported = true;
        }
        Ok(())
    }
}

pub struct Fans {
    pub fans: Vec<Fan>,
    /// False in dry runs: compute everything, write nothing.
    write: bool,
    /// The SMC reports a new fanN_manual with a delay, so the first check
    /// after taking the fans waits a full CHECK_EVERY.
    checked: Instant,
}

impl Fans {
    /// Finds the fans in the SMC directory; with `write`, takes them over.
    pub fn open(smc: &Path, write: bool) -> Result<Self> {
        let mut fans = Vec::new();
        for entry in std::fs::read_dir(smc)? {
            let file = entry?.file_name();
            let Some(name) = file
                .to_str()
                .and_then(|n| n.strip_suffix("_input"))
                .filter(|n| n.starts_with("fan"))
            else {
                continue;
            };
            let mut fan = Fan {
                base: smc.join(name),
                name: name.to_owned(),
                min: 0,
                max: 0,
                target: 0,
                written: None,
                slow_since: None,
                stall_reported: false,
            };
            fan.min = fan.read("_min")?;
            fan.max = fan.read("_max")?;
            ensure!(
                fan.min < fan.max,
                "{}: min {} is not below max {}",
                name,
                fan.min,
                fan.max
            );
            fans.push(fan);
        }
        ensure!(!fans.is_empty(), "no fans in {}", smc.display());
        fans.sort_by(|a, b| a.name.cmp(&b.name));

        let fans = Self {
            fans,
            write,
            checked: Instant::now(),
        };
        if write {
            for fan in &fans.fans {
                fan.write("_manual", 1)?;
            }
        }
        Ok(fans)
    }

    /// Sets every fan to `percent` of its own RPM range, writing only changes.
    pub fn set(&mut self, percent: f64, now: Instant) -> Result<()> {
        let check = self.write
            && self
                .checked
                .checked_add(CHECK_EVERY)
                .is_some_and(|due| now >= due);
        if check {
            self.checked = now;
        }
        for fan in &mut self.fans {
            fan.target = fan.rpm_for(percent);
            if !self.write {
                continue;
            }
            if check {
                fan.check(now)?;
            }
            if fan.written != Some(fan.target) {
                fan.write("_output", fan.target)?;
                fan.written = Some(fan.target);
            }
        }
        Ok(())
    }

    /// Takes every fan back at once, e.g. after the machine woke up.
    pub fn reclaim(&mut self) -> Result<()> {
        self.checked = Instant::now();
        if self.write {
            for fan in &mut self.fans {
                fan.write("_manual", 1)?;
                fan.written = None;
            }
        }
        Ok(())
    }
}

impl Drop for Fans {
    /// Hands the fans back to the SMC on a normal stop, an error or a panic.
    fn drop(&mut self) {
        if self.write {
            for fan in &self.fans {
                if let Err(err) = fan.write("_manual", 0) {
                    eprintln!("{}: {err:#}", fan.name);
                }
            }
        }
    }
}
