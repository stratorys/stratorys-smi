use std::path::{
    Path,
    PathBuf,
};
use std::time::Instant;
use std::{
    fs,
    io,
};

use super::read_bounded;
use crate::model::{
    Cause,
    Diagnostic,
    Operation,
    Problem,
    Reading,
    SensorCategory,
    Snapshot,
    SourceId,
    Threshold,
    Unit,
    limits,
};
use crate::source::{
    REDISCOVERY,
    Source,
};

struct SensorPath {
    input: PathBuf,
    alarm: PathBuf,
    template: Reading,
}

fn unit(prefix: &str) -> Option<Unit> {
    [
        ("temp", Unit::Celsius),
        ("fan", Unit::Rpm),
        ("in", Unit::Volts),
        ("power", Unit::Watts),
    ]
    .into_iter()
    .find_map(|(family, unit)| prefix.starts_with(family).then_some(unit))
}
fn read_value(
    path: &Path,
    unit: Unit,
) -> Option<f64> {
    let divisor = match unit {
        Unit::Celsius | Unit::Volts => 1000.0,
        Unit::Watts => 1_000_000.0,
        Unit::Rpm => 1.0,
    };
    Some(read_bounded(path).ok()?.parse::<f64>().ok()? / divisor)
}
fn alarm(path: &Path) -> bool { read_bounded(path).is_ok_and(|value| value == "1") }

fn sensor(
    directory: &Path,
    chip: &str,
    filename: &str,
    source: SourceId,
) -> Option<SensorPath> {
    let prefix = filename.strip_suffix("_input")?;
    let unit = unit(prefix)?;
    let input = directory.join(filename);
    let alarm_path = directory.join(format!("{prefix}_alarm"));
    let label = read_bounded(directory.join(format!("{prefix}_label")))
        .ok()
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| prefix.into());
    let limit = |suffix| {
        (unit == Unit::Celsius)
            .then(|| read_value(&directory.join(format!("{prefix}_{suffix}")), unit))
            .flatten()
            .and_then(|value| Threshold::try_from(value).ok())
    };
    let category = if unit == Unit::Rpm {
        SensorCategory::Fan
    } else {
        category(chip, &label)
    };
    let mut template = Reading::new(
        format!(
            "{} / {}",
            chip.chars().take(64).collect::<String>(),
            label.chars().take(96).collect::<String>()
        ),
        read_value(&input, unit)?,
        unit,
        category,
        source,
    )
    .ok()?;
    template.high = limit("max");
    template.critical = limit("crit");
    template.alarm = alarm(&alarm_path);
    Some(SensorPath {
        input,
        alarm: alarm_path,
        template,
    })
}

fn discover(
    root: &Path,
    chips: Chips,
) -> io::Result<Vec<SensorPath>> {
    Ok(fs::read_dir(root)?
        .take(128)
        .filter_map(Result::ok)
        .flat_map(|entry| {
            let directory = entry.path();
            let chip = read_bounded(directory.join("name"))
                .unwrap_or_else(|_| entry.file_name().to_string_lossy().into_owned());
            if (chip == "drivetemp") != (chips == Chips::Drives) {
                return Vec::new();
            }
            fs::read_dir(&directory)
                .into_iter()
                .flatten()
                .take(512)
                .filter_map(Result::ok)
                .filter_map(|file| {
                    sensor(
                        &directory,
                        &chip,
                        &file.file_name().to_string_lossy(),
                        chips.source(),
                    )
                })
                .collect()
        })
        .take(limits::READINGS)
        .collect())
}

fn category(
    chip: &str,
    label: &str,
) -> SensorCategory {
    let identity = format!("{chip} {label}").to_lowercase();
    if identity.contains("gpu") {
        SensorCategory::Gpu
    } else if ["coretemp", "k10temp", "cpu", "soc die"]
        .iter()
        .any(|name| identity.contains(name))
    {
        SensorCategory::Cpu
    } else if identity.contains("system") {
        SensorCategory::System
    } else {
        SensorCategory::Other
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Chips {
    Hardware,
    Drives,
}

pub struct Sensors {
    root: PathBuf,
    chips: Chips,
    paths: Vec<SensorPath>,
    discovered: Option<Instant>,
    notice: Option<Diagnostic>,
}
impl Sensors {
    pub fn new(chips: Chips) -> Self {
        Self {
            root: PathBuf::from("/sys/class/hwmon"),
            chips,
            paths: Vec::new(),
            discovered: None,
            notice: None,
        }
    }
    fn source(&self) -> SourceId { self.chips.source() }
}
impl Chips {
    fn source(self) -> SourceId {
        match self {
            Chips::Hardware => SourceId::Hwmon,
            Chips::Drives => SourceId::DriveTemp,
        }
    }
}
impl Source for Sensors {
    fn sample(
        &mut self,
        snapshot: &mut Snapshot,
    ) {
        if self
            .discovered
            .is_none_or(|time| time.elapsed() >= REDISCOVERY)
        {
            (self.paths, self.notice) = match discover(&self.root, self.chips) {
                Ok(paths) if paths.is_empty() => (
                    paths,
                    Some(Diagnostic::new(self.source(), Problem::NoReadings)),
                ),
                Ok(paths) => (paths, None),
                Err(error) => (
                    Vec::new(),
                    Some(Diagnostic::unavailable(
                        self.source(),
                        Operation::Discover,
                        Cause::of(&error).context(self.root.display()),
                    )),
                ),
            };
            self.discovered = Some(Instant::now());
        }
        let readings = self
            .paths
            .iter()
            .filter_map(|sensor| {
                let template = &sensor.template;
                let mut reading = Reading::new(
                    template.name.clone(),
                    read_value(&sensor.input, template.unit())?,
                    template.unit(),
                    template.category,
                    template.source,
                )
                .ok()?;
                reading.high = template.high;
                reading.critical = template.critical;
                reading.alarm = alarm(&sensor.alarm);
                Some(reading)
            })
            .collect::<Vec<_>>();
        if readings.len() < self.paths.len() {
            snapshot
                .notices
                .push(Diagnostic::new(self.source(), Problem::Partial));
        }
        snapshot.notices.extend(self.notice.clone());
        snapshot.readings.extend(readings);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        Chips,
        Sensors,
        discover,
    };
    use crate::model::{
        History,
        SensorCategory,
        Snapshot,
        SourceId,
        Unit,
    };
    use crate::source::Source;

    #[test]
    fn missing_sensors_fail_gracefully() {
        let mut sensors = Sensors::new(Chips::Hardware);
        for _ in 0..2 {
            let mut snapshot = Snapshot::default();
            sensors.sample(&mut snapshot);
            assert!(
                snapshot.readings.len() <= 1024,
                "Sensor readings must be bounded"
            );
        }
        assert!(sensors.paths.len() <= 1024, "Cached paths must be bounded");
    }
    #[test]
    fn fixtures() -> std::io::Result<()> {
        let root =
            std::env::temp_dir().join(format!("stratorys-smi-sensors-{}", std::process::id()));
        let chip = root.join("hwmon99");
        fs::create_dir_all(&chip)?;
        [
            ("name", "nvme"),
            ("temp1_input", "39850"),
            ("temp1_max", "65261850"),
            ("temp1_label", "Composite"),
            ("fan1_input", "0"),
            ("temp2_input", "broken"),
        ]
        .into_iter()
        .try_for_each(|(name, value)| fs::write(chip.join(name), value))?;
        let values = discover(&root, Chips::Hardware)?
            .into_iter()
            .map(|sensor| sensor.template)
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 2);
        assert!(
            values
                .iter()
                .any(|r| r.value() == 39.85 && r.high.is_none())
        );
        assert!(values.iter().any(|r| r.value() == 0.0
            && r.unit() == Unit::Rpm
            && r.category == SensorCategory::Fan));
        assert!(
            discover(&root, Chips::Drives)?.is_empty(),
            "Disk-only discovery must skip other chips"
        );
        fs::write(chip.join("name"), "drivetemp")?;
        let mut drives = Sensors::new(Chips::Drives);
        drives.root = root.clone();
        let mut snapshot = Snapshot::default();
        drives.sample(&mut snapshot);
        assert_eq!(snapshot.readings.len(), 2);
        assert!(
            snapshot
                .readings
                .iter()
                .all(|reading| reading.source == SourceId::DriveTemp),
            "SATA readings must use the same identity as their source"
        );
        let mut history = History::new();
        history.update(std::time::Instant::now(), &snapshot.readings);
        assert_eq!(
            history.maximum("drivetemp / Composite", SourceId::DriveTemp),
            Some(39.85)
        );
        assert_eq!(
            history.maximum("drivetemp / Composite", SourceId::Hwmon),
            None
        );
        assert!(
            discover(&root, Chips::Hardware)?.is_empty(),
            "Hardware discovery must skip SATA sensors"
        );
        fs::remove_dir_all(root)
    }
}
