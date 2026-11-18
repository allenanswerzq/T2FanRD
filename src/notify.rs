//! Runs a command when the fans speed up past a threshold, so whoever is at
//! the machine learns why it got loud.

use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::config::NotifyConfig;

pub struct Notifier {
    config: NotifyConfig,
    /// Whether the fans were at or above the threshold last time.
    above: bool,
    /// When the command last ran.
    last: Option<Instant>,
}

impl Notifier {
    pub fn new(config: NotifyConfig) -> Self {
        Self {
            config,
            above: false,
            last: None,
        }
    }

    /// Runs the command when `percent` crosses the threshold, at most once
    /// per `again_after`. `reason` names what drives the fans, and
    /// `curves` lists every curve; both are only built when needed.
    pub fn update(
        &mut self,
        now: Instant,
        percent: f64,
        reason: impl FnOnce() -> String,
        curves: impl FnOnce() -> String,
    ) {
        if self.crossed(now, percent) {
            self.run(percent, &reason(), &curves());
        }
    }

    fn crossed(&mut self, now: Instant, percent: f64) -> bool {
        let was_above = std::mem::replace(&mut self.above, percent >= self.config.above);
        let again_after = Duration::from_secs_f64(self.config.again_after);
        if !self.above || was_above || self.last.is_some_and(|t| now - t < again_after) {
            return false;
        }
        self.last = Some(now);
        true
    }

    fn run(&self, percent: f64, reason: &str, curves: &str) {
        let (program, args) = self
            .config
            .command
            .split_first()
            .expect("validated as not empty");
        let child = Command::new(program)
            .args(args)
            .env("T2FANRD_PERCENT", format!("{percent:.0}"))
            .env("T2FANRD_REASON", reason)
            .env("T2FANRD_CURVES", curves)
            .stdin(Stdio::null())
            .spawn();
        match child {
            // Wait elsewhere, so the fans never wait on the command.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(err) => eprintln!("Cannot run notify command {program}: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notifier() -> Notifier {
        Notifier::new(NotifyConfig {
            above: 80.0,
            again_after: 60.0,
            command: vec!["true".into()],
        })
    }

    #[test]
    fn runs_once_when_the_fans_cross_the_threshold() {
        let mut n = notifier();
        let t = Instant::now();
        assert!(!n.crossed(t, 50.0));
        assert!(n.crossed(t, 80.0));
        assert!(!n.crossed(t + Duration::from_secs(1), 100.0));
    }

    #[test]
    fn waits_again_after_before_running_again() {
        let mut n = notifier();
        let t = Instant::now();
        assert!(n.crossed(t, 90.0));
        assert!(!n.crossed(t + Duration::from_secs(10), 50.0));
        assert!(!n.crossed(t + Duration::from_secs(20), 90.0));
        assert!(!n.crossed(t + Duration::from_secs(70), 50.0));
        assert!(n.crossed(t + Duration::from_secs(80), 90.0));
    }
}
