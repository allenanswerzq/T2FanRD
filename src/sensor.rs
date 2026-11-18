//! Temperature sensors, found by chip and label so that the hwmonN numbers,
//! which change between boots, don't matter. The T2's SMC sensors sit on the
//! APP0001 ACPI device without an hwmon name; they are the chip `smc`.

use std::{
    fs::File,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

pub struct Sensor {
    pub name: String,
    file: File,
}

impl Sensor {
    /// Opens `chip:label`, where label is the sensor's label or its file
    /// name (`temp1`).
    pub fn open(spec: &str) -> Result<Self> {
        let Some((chip, label)) = spec.split_once(':') else {
            bail!("sensor {spec}: expected chip:label");
        };
        let dir = chips()?
            .into_iter()
            .find(|(name, _)| name == chip)
            .map(|(_, dir)| dir)
            .with_context(|| format!("sensor {spec}: no chip {chip}"))?;
        let input = temps(&dir)
            .into_iter()
            .find(|(l, stem, _)| l == label || stem == label)
            .map(|(_, _, input)| input)
            .with_context(|| format!("sensor {spec}: chip {chip} has no {label}"))?;
        let sensor = Self {
            name: spec.to_owned(),
            file: File::open(&input).with_context(|| format!("cannot open {}", input.display()))?,
        };
        sensor
            .read()
            .with_context(|| format!("cannot read {}", input.display()))?;
        Ok(sensor)
    }

    /// The temperature in degrees Celsius.
    pub fn read(&self) -> std::io::Result<f64> {
        let mut buf = [0u8; 16];
        let n = self.file.read_at(&mut buf, 0)?;
        std::str::from_utf8(&buf[..n])
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|milli| milli as f64 / 1000.0)
            .ok_or_else(|| std::io::Error::other("not a temperature"))
    }
}

/// Every sensor a curve can use, as `chip:label`, and its temperature.
pub fn list() -> Result<Vec<(String, Option<f64>)>> {
    let mut found = Vec::new();
    for (chip, dir) in chips()? {
        for (label, _, input) in temps(&dir) {
            let temp = std::fs::read_to_string(input)
                .ok()
                .and_then(|t| t.trim().parse::<f64>().ok())
                .map(|milli| milli / 1000.0);
            found.push((format!("{chip}:{label}"), temp));
        }
    }
    Ok(found)
}

/// The T2's SMC device, which has the fans and the SMC sensors.
pub fn smc_dir() -> Result<PathBuf> {
    let devices = Path::new("/sys/bus/acpi/devices");
    std::fs::read_dir(devices)
        .with_context(|| format!("cannot list {}", devices.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("APP0001"))
                && p.join("fan1_input").exists()
        })
        .context("no T2 SMC (APP0001) with fans found")
}

/// Chip names and their directories: the SMC, then every named hwmon chip.
fn chips() -> Result<Vec<(String, PathBuf)>> {
    let mut chips = Vec::new();
    if let Ok(dir) = smc_dir() {
        chips.push(("smc".to_owned(), dir));
    }
    for entry in std::fs::read_dir("/sys/class/hwmon").context("cannot list hwmon")? {
        let dir = entry?.path();
        if let Ok(name) = std::fs::read_to_string(dir.join("name")) {
            chips.push((name.trim().to_owned(), dir));
        }
    }
    Ok(chips)
}

/// The temperature inputs of a chip: label (or file name), file name, path.
fn temps(dir: &Path) -> Vec<(String, String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut temps: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let file = e.file_name().into_string().ok()?;
            let stem = file.strip_suffix("_input")?;
            stem.starts_with("temp").then_some(())?;
            let label = std::fs::read_to_string(dir.join(format!("{stem}_label")))
                .map_or_else(|_| stem.to_owned(), |l| l.trim().to_owned());
            Some((label, stem.to_owned(), e.path()))
        })
        .collect();
    temps.sort_by_key(|(_, stem, _)| stem[4..].parse::<u32>().unwrap_or(u32::MAX));
    temps
}
