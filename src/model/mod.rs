mod diagnostic;
mod history;
mod measurement;

use std::time::Duration;

pub use self::diagnostic::{
    Cause,
    Diagnostic,
    Operation,
    Problem,
    SourceId,
    TelemetryState,
    deduplicate,
};
pub use self::history::History;
pub use self::measurement::{
    Bytes,
    FrequencyMhz,
    GpuMemory,
    Measurement,
    MeasurementError,
    MemoryUsage,
    Percentage,
    PowerWatts,
    TemperatureCelsius,
    Threshold,
};

pub mod limits {
    pub const READINGS: usize = 1024;
    pub const CORES: usize = 256;
    pub const GPUS: usize = 32;
    pub const NOTICES: usize = 64;
    pub const STORAGE: usize = 32;
    pub const NAME_CHARS: usize = 160;
    pub const HISTORY_SAMPLES: usize = 601;
    pub const HISTORY_SECONDS: f64 = 120.0;
}

#[derive(Default)]
pub struct SystemInfo {
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub architecture: String,
    pub cpu: Option<String>,
    pub cores_physical: Option<usize>,
    pub threads: usize,
    pub memory_bytes: Bytes,
    pub motherboard: Option<String>,
    pub storage: Vec<StorageInfo>,
}
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct StorageInfo {
    pub name: String,
    pub model: Option<String>,
    pub bytes: Bytes,
}
#[derive(Default)]
pub struct Uptime {
    pub host: Option<Duration>,
    pub session: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unit {
    Celsius,
    Rpm,
    Volts,
    Watts,
}
impl Unit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Celsius => "°C",
            Self::Rpm => "RPM",
            Self::Volts => "V",
            Self::Watts => "W",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorCategory {
    Cpu,
    Gpu,
    System,
    Fan,
    Other,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MemoryTopology {
    #[default]
    Dedicated,
    Unified,
}

#[derive(Clone, Debug)]
pub struct Reading {
    pub name: String,
    value: f64,
    unit: Unit,
    pub category: SensorCategory,
    pub source: SourceId,
    pub high: Option<Threshold>,
    pub critical: Option<Threshold>,
    pub alarm: bool,
}
impl Reading {
    pub fn new(
        name: String,
        value: f64,
        unit: Unit,
        category: SensorCategory,
        source: SourceId,
    ) -> Result<Self, MeasurementError> {
        let valid = match unit {
            Unit::Celsius => valid_temperature(value),
            Unit::Rpm | Unit::Volts | Unit::Watts => value.is_finite() && value >= 0.0,
        };
        if !valid {
            return Err(MeasurementError::sensor(value));
        }
        Ok(Self {
            name: name.chars().take(limits::NAME_CHARS).collect(),
            value,
            unit,
            category,
            source,
            high: None,
            critical: None,
            alarm: false,
        })
    }
    pub fn unit(&self) -> Unit { self.unit }
    pub fn value(&self) -> f64 { self.value }
    pub fn severity(&self) -> Severity {
        let reached =
            |limit: Option<Threshold>| limit.is_some_and(|limit| self.value >= f64::from(limit));
        if reached(self.critical) {
            Severity::Critical
        } else if reached(self.high) {
            Severity::Warning
        } else {
            Severity::Normal
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Severity {
    Normal,
    Warning,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransferRate {
    pub incoming: f64,
    pub outgoing: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CpuClocks {
    pub efficiency_mhz: Measurement<FrequencyMhz>,
    pub performance_mhz: Measurement<FrequencyMhz>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvailableKind {
    Available,
    Noncompressed,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvailableMemory {
    pub bytes: Bytes,
    pub kind: AvailableKind,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub cpu_usage: Measurement<Percentage>,
    pub cpu_power_watts: Measurement<PowerWatts>,
    pub cpu_clocks_mhz: Measurement<CpuClocks>,
    pub load_average: Option<LoadAverage>,
    pub uptime: Option<Duration>,
    pub cores: Vec<Measurement<Percentage>>,
    pub memory_bytes: Measurement<MemoryUsage>,
    pub memory_available: Measurement<AvailableMemory>,
    pub swap_bytes: Measurement<MemoryUsage>,
    pub readings: Vec<Reading>,
    pub gpu: Vec<GpuSample>,
    pub host: Option<HostSample>,
    pub notices: Vec<Diagnostic>,
}

#[derive(Clone, Default)]
pub struct GpuSample {
    pub identity: String,
    pub name: String,
    pub memory_topology: MemoryTopology,
    pub activity_percent: Measurement<Percentage>,
    pub memory_activity_percent: Measurement<Percentage>,
    pub memory_bytes: Measurement<GpuMemory>,
    pub core_mhz: Measurement<FrequencyMhz>,
    pub memory_mhz: Measurement<FrequencyMhz>,
    pub power_watts: Measurement<PowerWatts>,
    pub power_limit_watts: Measurement<PowerWatts>,
    pub temperature_celsius: Measurement<TemperatureCelsius>,
    pub slowdown_celsius: Measurement<TemperatureCelsius>,
    pub fan_percent: Measurement<Percentage>,
    pub performance_state: Option<String>,
    pub clock_limits: Option<String>,
}
impl GpuSample {
    fn expire(
        self,
        state: TelemetryState,
    ) -> Self {
        let Self {
            identity,
            name,
            memory_topology,
            activity_percent,
            memory_activity_percent,
            memory_bytes,
            core_mhz,
            memory_mhz,
            power_watts,
            power_limit_watts,
            temperature_celsius,
            slowdown_celsius,
            fan_percent,
            performance_state,
            clock_limits,
        } = self;
        Self {
            identity,
            name,
            memory_topology,
            activity_percent: activity_percent.expire(state),
            memory_activity_percent: memory_activity_percent.expire(state),
            memory_bytes: memory_bytes.expire(state),
            core_mhz: core_mhz.expire(state),
            memory_mhz: memory_mhz.expire(state),
            power_watts: power_watts.expire(state),
            power_limit_watts: power_limit_watts.expire(state),
            temperature_celsius: temperature_celsius.expire(state),
            slowdown_celsius: slowdown_celsius.expire(state),
            fan_percent: fan_percent.expire(state),
            performance_state,
            clock_limits,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct HostSample {
    pub swap_pages_per_second: Measurement<TransferRate>,
    pub disk_bytes_per_second: Measurement<TransferRate>,
    pub io_wait_percent: Measurement<Percentage>,
    pub pressure_percent: Pressure,
}
#[derive(Clone, Copy, Default)]
pub struct Pressure {
    pub cpu: Measurement<Percentage>,
    pub memory: Measurement<Percentage>,
    pub io: Measurement<Percentage>,
}

#[derive(Clone, Copy)]
pub enum GpuMetric {
    Activity,
    Temperature,
    Power,
}

impl Snapshot {
    pub fn merge(
        &mut self,
        part: Self,
    ) {
        let Self {
            cpu_usage,
            cpu_power_watts,
            cpu_clocks_mhz,
            load_average,
            uptime,
            cores,
            memory_bytes,
            memory_available,
            swap_bytes,
            readings,
            gpu,
            host,
            notices,
        } = part;
        self.cpu_usage = self.cpu_usage.or(cpu_usage);
        self.cpu_power_watts = self.cpu_power_watts.or(cpu_power_watts);
        self.cpu_clocks_mhz = self.cpu_clocks_mhz.or(cpu_clocks_mhz);
        self.load_average = self.load_average.or(load_average);
        self.uptime = self.uptime.or(uptime);
        if self.cores.is_empty() {
            self.cores = cores;
        }
        self.memory_bytes = self.memory_bytes.or(memory_bytes);
        self.memory_available = self.memory_available.or(memory_available);
        self.swap_bytes = self.swap_bytes.or(swap_bytes);
        self.readings.extend(readings);
        self.gpu.extend(gpu);
        self.host = self.host.or(host);
        self.notices.extend(notices);
    }
    pub fn expire(
        self,
        state: TelemetryState,
    ) -> Self {
        let Self {
            cpu_usage,
            cpu_power_watts,
            cpu_clocks_mhz,
            load_average,
            uptime,
            cores,
            memory_bytes,
            memory_available,
            swap_bytes,
            readings,
            gpu,
            host,
            notices,
        } = self;
        drop(readings);
        Self {
            cpu_usage: cpu_usage.expire(state),
            cpu_power_watts: cpu_power_watts.expire(state),
            cpu_clocks_mhz: cpu_clocks_mhz.expire(state),
            load_average,
            uptime,
            cores: cores.into_iter().map(|core| core.expire(state)).collect(),
            memory_bytes: memory_bytes.expire(state),
            memory_available: memory_available.expire(state),
            swap_bytes: swap_bytes.expire(state),
            readings: Vec::new(),
            gpu: gpu.into_iter().map(|gpu| gpu.expire(state)).collect(),
            host: host.map(|host| HostSample {
                swap_pages_per_second: host.swap_pages_per_second.expire(state),
                disk_bytes_per_second: host.disk_bytes_per_second.expire(state),
                io_wait_percent: host.io_wait_percent.expire(state),
                pressure_percent: Pressure {
                    cpu: host.pressure_percent.cpu.expire(state),
                    memory: host.pressure_percent.memory.expire(state),
                    io: host.pressure_percent.io.expire(state),
                },
            }),
            notices,
        }
    }
    pub fn hottest(
        &self,
        category: SensorCategory,
    ) -> Option<f64> {
        hottest(&self.readings, category)
    }
    pub fn sensor_power(
        &self,
        category: SensorCategory,
    ) -> Option<f64> {
        self.readings
            .iter()
            .find(|reading| reading.category == category && reading.unit == Unit::Watts)
            .map(Reading::value)
    }
    pub fn cpu_power(&self) -> Option<f64> {
        self.cpu_power_watts
            .numeric()
            .or_else(|| self.sensor_power(SensorCategory::Cpu))
    }
    pub fn peak_core(&self) -> Option<f64> {
        self.cores
            .iter()
            .filter_map(|core| core.numeric())
            .reduce(f64::max)
    }
    pub fn fans(&self) -> impl Iterator<Item = &Reading> {
        self.readings
            .iter()
            .filter(|reading| reading.unit == Unit::Rpm)
    }
}
pub fn hottest(
    readings: &[Reading],
    category: SensorCategory,
) -> Option<f64> {
    readings
        .iter()
        .filter(|reading| reading.unit == Unit::Celsius && reading.category == category)
        .map(Reading::value)
        .reduce(f64::max)
}

pub fn valid_temperature(value: f64) -> bool {
    value.is_finite() && (-40.0..=200.0).contains(&value)
}

#[cfg(test)]
mod tests {
    use super::{
        GpuSample,
        Measurement,
        Reading,
        SensorCategory,
        Severity,
        Snapshot,
        SourceId,
        TelemetryState,
        Threshold,
        Unit,
    };

    #[test]
    fn colors_use_real_limits() -> Result<(), super::MeasurementError> {
        let reading = Reading {
            critical: Threshold::try_from(90.0).ok(),
            ..Reading::new(
                "CPU".into(),
                80.0,
                Unit::Celsius,
                SensorCategory::Cpu,
                SourceId::SENSOR,
            )?
        };
        assert_eq!(reading.severity(), Severity::Normal);
        assert_eq!(
            Reading {
                value: 95.0,
                ..reading
            }
            .severity(),
            Severity::Critical
        );
        Ok(())
    }
    #[test]
    fn readings_are_validated_and_bounded() -> Result<(), super::MeasurementError> {
        let bounded = Reading::new(
            "x".repeat(1000),
            0.0,
            Unit::Watts,
            SensorCategory::Other,
            SourceId::SENSOR,
        )?;
        assert_eq!(bounded.name.chars().count(), 160);
        assert!(
            Reading::new(
                "invalid".into(),
                -1.0,
                Unit::Watts,
                SensorCategory::Other,
                SourceId::SENSOR
            )
            .is_err(),
            "Negative sensor power must be unavailable"
        );
        Ok(())
    }
    #[test]
    fn merge_appends_lists_and_keeps_the_first_reported_value()
    -> Result<(), super::MeasurementError> {
        let reading = Reading::new(
            "CPU".into(),
            50.0,
            Unit::Celsius,
            SensorCategory::Cpu,
            SourceId::SENSOR,
        )?;
        let mut snapshot = Snapshot {
            cpu_usage: Measurement::validated(Some(10.0)),
            readings: vec![reading.clone()],
            ..Snapshot::default()
        };
        snapshot.merge(Snapshot {
            cpu_usage: Measurement::validated(Some(90.0)),
            cpu_power_watts: Measurement::validated(Some(3.0)),
            readings: vec![reading],
            gpu: vec![GpuSample::default()],
            ..Snapshot::default()
        });
        assert_eq!(snapshot.cpu_usage.numeric(), Some(10.0));
        assert_eq!(snapshot.cpu_power_watts.numeric(), Some(3.0));
        assert_eq!(snapshot.readings.len(), 2);
        assert_eq!(snapshot.gpu.len(), 1);
        Ok(())
    }
    #[test]
    fn expired_samples_keep_devices_but_no_values() -> Result<(), super::MeasurementError> {
        let snapshot = Snapshot {
            cpu_power_watts: Measurement::validated(Some(3.0)),
            readings: vec![Reading::new(
                "CPU".into(),
                50.0,
                Unit::Celsius,
                SensorCategory::Cpu,
                SourceId::SENSOR,
            )?],
            gpu: vec![GpuSample {
                name: "GPU".into(),
                activity_percent: Measurement::validated(Some(40.0)),
                ..GpuSample::default()
            }],
            ..Snapshot::default()
        }
        .expire(TelemetryState::Stale);
        assert_eq!(snapshot.cpu_power_watts, Measurement::Stale);
        assert!(snapshot.readings.is_empty(), "Stale sensors are not shown");
        assert_eq!(
            snapshot.gpu.first().map(|gpu| gpu.activity_percent),
            Some(Measurement::Stale)
        );
        Ok(())
    }
}
