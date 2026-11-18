//! A curve: the hottest of its sensors, averaged over time, held up by
//! hysteresis while it cools, and mapped linearly onto 0..100%.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::{config::CurveConfig, sensor::Sensor};

pub struct Curve {
    config: CurveConfig,
    inputs: Vec<Input>,
    /// Moving average of the hottest sensor, and when it was updated.
    average: Option<(Instant, f64)>,
    /// The temperature after hysteresis, which the curve maps.
    pub temp: f64,
    pub percent: f64,
}

struct Input {
    sensor: Sensor,
    last: Option<(Instant, f64)>,
    error: Option<std::io::Error>,
}

impl Curve {
    pub fn open(config: &CurveConfig) -> Result<Self> {
        let inputs = config
            .sensors
            .iter()
            .map(|spec| {
                Ok(Input {
                    sensor: Sensor::open(spec)?,
                    last: None,
                    error: None,
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            config: config.clone(),
            inputs,
            average: None,
            temp: f64::NAN,
            percent: 0.0,
        })
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Reads the sensors and returns the speed this curve asks for. A sensor
    /// without a good reading for `stale_after` fails the curve: a failed
    /// reading is never a temperature.
    pub fn update(&mut self, now: Instant, stale_after: Duration) -> Result<f64, String> {
        let mut hottest = f64::MIN;
        for input in &mut self.inputs {
            match input.sensor.read() {
                Ok(t) => {
                    input.last = Some((now, t));
                    input.error = None;
                }
                Err(err) => input.error = Some(err),
            }
            match input.last {
                Some((at, t)) if now.duration_since(at) <= stale_after => hottest = hottest.max(t),
                _ => {
                    let why = input
                        .error
                        .as_ref()
                        .map_or_else(|| "no reading".to_owned(), ToString::to_string);
                    return Err(format!("{}: {why}", input.sensor.name));
                }
            }
        }

        let average = match self.average {
            Some((at, avg)) => smooth(avg, hottest, now.duration_since(at), self.config.average),
            None => hottest,
        };
        self.average = Some((now, average));
        self.temp = hysteresis(self.temp, average, self.config.hysteresis);
        let ratio = (self.temp - self.config.low) / (self.config.high - self.config.low);
        self.percent = (ratio * 100.0).clamp(0.0, 100.0);
        Ok(self.percent)
    }

    /// Forgets the averages, e.g. after sleep, where they no longer apply.
    pub fn reset(&mut self) {
        self.average = None;
        self.temp = f64::NAN;
        for input in &mut self.inputs {
            input.last = None;
        }
    }
}

/// One step of an exponential moving average with time constant `tau`.
pub fn smooth(avg: f64, value: f64, dt: Duration, tau: f64) -> f64 {
    if tau <= 0.0 {
        return value;
    }
    avg + (1.0 - (-dt.as_secs_f64() / tau).exp()) * (value - avg)
}

/// fan2go's hysteresis.down: a rising temperature counts at once; a falling
/// one keeps the held value until it is more than `down` below it, and then
/// stays `down` above the measurement. NaN means nothing is held yet.
fn hysteresis(held: f64, measured: f64, down: f64) -> f64 {
    if measured < held {
        held.min(measured + down)
    } else {
        measured
    }
}

#[cfg(test)]
mod tests {
    use super::hysteresis;

    #[test]
    fn hysteresis_holds_then_follows_a_falling_temperature() {
        let mut t = hysteresis(f64::NAN, 60.0, 3.0);
        assert_eq!(t, 60.0);
        t = hysteresis(t, 58.0, 3.0);
        assert_eq!(t, 60.0);
        t = hysteresis(t, 55.0, 3.0);
        assert_eq!(t, 58.0);
        t = hysteresis(t, 59.0, 3.0);
        assert_eq!(t, 59.0);
    }
}
