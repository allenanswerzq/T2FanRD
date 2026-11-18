//! The config file: how often to update, how the fans move, and the curves
//! that turn temperatures into fan speed.

use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Seconds between updates.
    #[serde(default = "default_interval")]
    pub interval: f64,
    /// Seconds without a good reading before a sensor counts as failed and
    /// the fans go to full speed.
    #[serde(default = "default_stale_after")]
    pub stale_after: f64,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(rename = "curve")]
    pub curves: Vec<CurveConfig>,
    pub notify: Option<NotifyConfig>,
}

/// How the fan speed follows the hottest curve, in percent of each fan's
/// RPM range. Same model as fan2go's `direct` control algorithm.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputConfig {
    /// Low-pass time constant in seconds; 0 follows the curves directly.
    pub smoothing: f64,
    /// Changes smaller than this many percent are not written.
    pub deadband: f64,
    /// Percent per second the fans may speed up; 0 means at once.
    pub ramp_up: f64,
    /// Percent per second the fans may slow down; 0 means at once.
    pub ramp_down: f64,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            smoothing: 0.0,
            deadband: 1.0,
            ramp_up: 0.0,
            ramp_down: 3.0,
        }
    }
}

/// One temperature source and the speed it asks for: 0% at `low` and below,
/// 100% at `high` and above, a straight line between.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurveConfig {
    pub name: String,
    /// `chip:label`, as `t2fanrd --list-sensors` prints them. The curve uses
    /// the hottest one.
    pub sensors: Vec<String>,
    pub low: f64,
    pub high: f64,
    /// Time constant in seconds of the moving average; 0 uses each reading.
    #[serde(default)]
    pub average: f64,
    /// Degrees a temperature must fall before the curve follows it down.
    #[serde(default)]
    pub hysteresis: f64,
}

/// A command to run when the fans speed up past `above` percent. It gets
/// T2FANRD_PERCENT, T2FANRD_REASON (the curve that drives the fans, or the
/// failed sensor) and T2FANRD_CURVES (every curve) in its environment.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotifyConfig {
    pub above: f64,
    /// Seconds before the command may run again.
    #[serde(default = "default_again_after")]
    pub again_after: f64,
    /// The program and its arguments.
    pub command: Vec<String>,
}

fn default_interval() -> f64 {
    0.5
}

fn default_stale_after() -> f64 {
    5.0
}

fn default_again_after() -> f64 {
    600.0
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("invalid {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        ensure!(self.interval > 0.0, "interval must be above 0");
        ensure!(
            self.stale_after >= self.interval,
            "stale_after must be at least interval"
        );
        let o = self.output;
        ensure!(
            [o.smoothing, o.deadband, o.ramp_up, o.ramp_down]
                .iter()
                .all(|v| *v >= 0.0),
            "output values must not be negative"
        );
        ensure!(!self.curves.is_empty(), "add at least one [[curve]]");
        for c in &self.curves {
            ensure!(!c.sensors.is_empty(), "curve {} has no sensors", c.name);
            ensure!(c.low < c.high, "curve {}: low must be below high", c.name);
            ensure!(
                c.average >= 0.0 && c.hysteresis >= 0.0,
                "curve {}: average and hysteresis must not be negative",
                c.name
            );
        }
        if let Some(n) = &self.notify {
            ensure!(
                n.above > 0.0 && n.above <= 100.0,
                "notify.above must be above 0 and at most 100"
            );
            ensure!(
                n.again_after >= 0.0,
                "notify.again_after must not be negative"
            );
            ensure!(!n.command.is_empty(), "notify.command must name a program");
        }
        Ok(())
    }
}
