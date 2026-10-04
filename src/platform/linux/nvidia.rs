use std::time::Instant;

use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{
    Clock,
    TemperatureSensor,
    TemperatureThreshold,
};
use nvml_wrapper::error::NvmlError;

use crate::model::{
    Cause,
    Diagnostic,
    GpuMemory,
    GpuSample,
    Measurement,
    MemoryTopology,
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

struct DeviceInfo {
    index: u32,
    identity: String,
    name: String,
    high_celsius: Option<Threshold>,
    slowdown_celsius: Option<Threshold>,
}

pub struct Nvidia {
    nvml: Result<Nvml, Cause>,
    devices: Vec<DeviceInfo>,
    notice: Option<Diagnostic>,
    discovered: Instant,
}
impl Nvidia {
    pub fn new() -> Self {
        let mut nvidia = Self {
            nvml: initialize(),
            devices: Vec::new(),
            notice: None,
            discovered: Instant::now(),
        };
        nvidia.discover();
        nvidia
    }
    fn discover(&mut self) {
        self.devices = Vec::new();
        let nvml = match &self.nvml {
            Ok(nvml) => nvml,
            Err(cause) => {
                return self.notice = Some(Diagnostic::unavailable(
                    SourceId::Nvml,
                    Operation::Initialize,
                    cause.clone(),
                ));
            }
        };
        let count = match nvml.device_count() {
            Ok(count) => count,
            Err(error) => {
                return self.notice = Some(Diagnostic::unavailable(
                    SourceId::Nvml,
                    Operation::Discover,
                    Cause::of(&error),
                ));
            }
        };
        let threshold = |device: &nvml_wrapper::Device<'_>, kind| {
            device
                .temperature_threshold(kind)
                .ok()
                .and_then(|value| Threshold::try_from(f64::from(value)).ok())
        };
        self.devices = (0..count.min(limits::GPUS as u32))
            .filter_map(|index| {
                let device = nvml.device_by_index(index).ok()?;
                Some(DeviceInfo {
                    index,
                    identity: device.uuid().ok()?,
                    name: device.name().unwrap_or_else(|_| format!("NVIDIA {index}")),
                    high_celsius: threshold(&device, TemperatureThreshold::GpuMax),
                    slowdown_celsius: threshold(&device, TemperatureThreshold::Slowdown),
                })
            })
            .collect();
        self.notice = u32::try_from(self.devices.len())
            .is_ok_and(|length| length < count)
            .then(|| {
                Diagnostic::new(
                    SourceId::Nvml,
                    Problem::Limit {
                        max: limits::GPUS,
                    },
                )
            });
    }
}
fn initialize() -> Result<Nvml, Cause> { Nvml::init().map_err(|error| Cause::of(&error)) }

impl Source for Nvidia {
    fn sample(
        &mut self,
        snapshot: &mut Snapshot,
    ) {
        if self.discovered.elapsed() >= REDISCOVERY {
            if self.nvml.is_err() {
                self.nvml = initialize();
            }
            self.discover();
            self.discovered = Instant::now();
        }
        snapshot.notices.extend(self.notice.clone());
        let Ok(nvml) = &self.nvml else {
            return;
        };
        for info in &self.devices {
            match sample(nvml, info) {
                Ok((gpu, reading)) => {
                    snapshot.readings.extend(reading);
                    snapshot.gpu.push(gpu);
                }
                Err(error) => snapshot.notices.push(Diagnostic::unavailable(
                    SourceId::Nvml,
                    Operation::Sample,
                    Cause::of(&error).context(format!("GPU {}", info.index)),
                )),
            }
        }
    }
}

fn sample(
    nvml: &Nvml,
    info: &DeviceInfo,
) -> Result<(GpuSample, Option<Reading>), NvmlError> {
    let device = nvml.device_by_index(info.index)?;
    let usage = device.utilization_rates().ok();
    let milliwatts =
        |value: Result<u32, NvmlError>| value.ok().map(|value| f64::from(value) / 1000.0);
    let gpu = GpuSample {
        identity: info.identity.clone(),
        name: info.name.clone(),
        memory_topology: MemoryTopology::Dedicated,
        activity_percent: Measurement::validated(usage.as_ref().map(|value| f64::from(value.gpu))),
        memory_activity_percent: Measurement::validated(
            usage.as_ref().map(|value| f64::from(value.memory)),
        ),
        memory_bytes: device
            .memory_info()
            .ok()
            .and_then(|value| {
                GpuMemory::try_from((value.used, value.reserved, value.free, value.total)).ok()
            })
            .into(),
        core_mhz: Measurement::validated(device.clock_info(Clock::Graphics).ok()),
        memory_mhz: Measurement::validated(device.clock_info(Clock::Memory).ok()),
        power_watts: Measurement::validated(milliwatts(device.power_usage())),
        power_limit_watts: Measurement::validated(milliwatts(device.enforced_power_limit())),
        temperature_celsius: Measurement::validated(
            device
                .temperature(TemperatureSensor::Gpu)
                .ok()
                .map(f64::from),
        ),
        slowdown_celsius: Measurement::validated(info.slowdown_celsius.map(f64::from)),
        fan_percent: Measurement::validated(device.fan_speed(0).ok().map(f64::from)),
        performance_state: device
            .performance_state()
            .ok()
            .map(|value| format!("{value:?}")),
        clock_limits: device.current_throttle_reasons().ok().map(|reasons| {
            if reasons.is_empty() {
                "None".into()
            } else {
                reasons
                    .iter_names()
                    .map(|(name, _)| name.replace('_', " ").to_lowercase())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }),
    };
    let reading = gpu.temperature_celsius.numeric().and_then(|value| {
        Reading::new(
            format!("GPU / {}", gpu.identity),
            value,
            Unit::Celsius,
            SensorCategory::Gpu,
            SourceId::Nvml,
        )
        .ok()
        .map(|mut reading| {
            reading.high = info.high_celsius;
            reading.critical = info.slowdown_celsius;
            reading
        })
    });
    Ok((gpu, reading))
}
