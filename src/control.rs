//! Turns the speed the hottest curve asks for into the speed the fans get,
//! like fan2go's `direct` algorithm: a low-pass filter, a deadband, and
//! separate ramp rates up and down. Values are percent of the fan range.

use std::time::Instant;

use crate::{config::OutputConfig, curve::smooth};

pub struct Control {
    config: OutputConfig,
    state: Option<State>,
}

struct State {
    at: Instant,
    filtered: f64,
    output: f64,
}

impl Control {
    pub fn new(config: OutputConfig) -> Self {
        Self {
            config,
            state: None,
        }
    }

    pub fn update(&mut self, now: Instant, target: f64) -> f64 {
        let target = target.clamp(0.0, 100.0);
        let Some(s) = &mut self.state else {
            self.hold(now, target);
            return target;
        };
        let dt = now.saturating_duration_since(s.at);
        s.at = now;

        s.filtered = smooth(s.filtered, target, dt, self.config.smoothing);
        if (s.filtered - target).abs() < 0.1 {
            s.filtered = target;
        }

        let delta = s.filtered - s.output;
        // Small changes are not worth a write, except reaching either end.
        let at_end = s.filtered <= 0.0 || s.filtered >= 100.0;
        if delta.abs() < self.config.deadband && !at_end {
            return s.output;
        }
        // The filter lags; never move away from where the curves are now.
        if delta * (target - s.output) < 0.0 {
            return s.output;
        }

        let secs = dt.as_secs_f64();
        let step = if delta > 0.0 && self.config.ramp_up > 0.0 {
            delta.min(self.config.ramp_up * secs)
        } else if delta < 0.0 && self.config.ramp_down > 0.0 {
            delta.max(-self.config.ramp_down * secs)
        } else {
            delta
        };
        s.output = (s.output + step).clamp(0.0, 100.0);
        s.output
    }

    /// Continues from `value`, e.g. full speed after a sensor failure, so
    /// the fans ramp down from there once the sensors work again.
    pub fn hold(&mut self, now: Instant, value: f64) {
        self.state = Some(State {
            at: now,
            filtered: value,
            output: value,
        });
    }

    pub fn reset(&mut self) {
        self.state = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn control(smoothing: f64, ramp_down: f64) -> Control {
        Control::new(OutputConfig {
            smoothing,
            deadband: 1.0,
            ramp_up: 0.0,
            ramp_down,
        })
    }

    #[test]
    fn speeds_up_at_once_and_slows_down_at_the_ramp_rate() {
        let t0 = Instant::now();
        let mut c = control(0.0, 3.0);
        assert_eq!(c.update(t0, 20.0), 20.0);
        assert_eq!(c.update(t0 + Duration::from_secs(1), 90.0), 90.0);
        let after_10s = c.update(t0 + Duration::from_secs(11), 0.0);
        assert!((after_10s - 60.0).abs() < 1e-9, "{after_10s}");
        assert_eq!(c.update(t0 + Duration::from_secs(60), 0.0), 0.0);
    }

    #[test]
    fn ignores_changes_inside_the_deadband() {
        let t0 = Instant::now();
        let mut c = control(0.0, 0.0);
        c.update(t0, 50.0);
        assert_eq!(c.update(t0 + Duration::from_secs(1), 50.6), 50.0);
        assert_eq!(c.update(t0 + Duration::from_secs(2), 51.5), 51.5);
    }

    #[test]
    fn smoothing_follows_with_its_time_constant() {
        let t0 = Instant::now();
        let mut c = control(2.0, 0.0);
        c.update(t0, 0.0);
        let one_tau = c.update(t0 + Duration::from_secs(2), 100.0);
        assert!((one_tau - 63.2).abs() < 0.1, "{one_tau}");
    }

    #[test]
    fn ramps_down_from_a_held_value() {
        let t0 = Instant::now();
        let mut c = control(0.0, 3.0);
        c.update(t0, 10.0);
        c.hold(t0, 100.0);
        assert_eq!(c.update(t0 + Duration::from_secs(1), 10.0), 97.0);
    }
}
