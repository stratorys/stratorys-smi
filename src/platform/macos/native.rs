use std::collections::BTreeSet;
use std::panic::{
    self,
    AssertUnwindSafe,
};
use std::time::{
    Duration,
    Instant,
};

use macmon::sources::{
    IOHIDSensors,
    SMC,
};

use super::ioreport::{
    RuntimeMetrics,
    Telemetry,
};
use super::smc::{
    SENSOR_REFRESH,
    SensorBatch,
    SmcSensors,
};
use crate::model::{
    Cause,
    CpuClocks,
    Diagnostic,
    GpuSample,
    Measurement,
    Operation,
    Problem,
    Reading,
    SensorCategory,
    Snapshot,
    SourceId,
    Unit,
    hottest,
};
use crate::source::{
    REDISCOVERY,
    Source,
};

#[derive(Default)]
enum Runtime {
    #[default]
    Ready,
    Retry {
        at: Instant,
        notice: Diagnostic,
    },
    Stopped(Diagnostic),
}
impl Runtime {
    fn waiting(
        &self,
        now: Instant,
    ) -> Option<&Diagnostic> {
        match self {
            Self::Ready => None,
            Self::Retry {
                at,
                notice,
            } => (now < *at).then_some(notice),
            Self::Stopped(notice) => Some(notice),
        }
    }
    fn attempt<T>(
        &mut self,
        now: Instant,
        sample: impl FnOnce(bool) -> Result<T, Box<dyn std::error::Error>>,
    ) -> Result<T, Diagnostic> {
        if let Some(notice) = self.waiting(now) {
            return Err(notice.clone());
        }
        let reset = matches!(self, Self::Retry { .. });
        match panic::catch_unwind(AssertUnwindSafe(|| sample(reset))) {
            Ok(Ok(value)) => {
                *self = Self::Ready;
                Ok(value)
            }
            Ok(Err(error)) => {
                let notice = Diagnostic::unavailable(
                    SourceId::IoReport,
                    Operation::Sample,
                    Cause::of(error.as_ref()),
                );
                *self = Self::Retry {
                    at: now + REDISCOVERY,
                    notice: notice.clone(),
                };
                Err(notice)
            }
            Err(_) => {
                let notice = Diagnostic::new(SourceId::IoReport, Problem::Panicked);
                *self = Self::Stopped(notice.clone());
                Err(notice)
            }
        }
    }
}

const WINDOW: Duration = Duration::from_secs(1);

pub struct Native {
    telemetry: Result<Telemetry, Diagnostic>,
    runtime: Runtime,
    smc: Result<SMC, Diagnostic>,
    hid: HidSensors,
    smc_sensors: SmcSensors,
    gpu: Option<GpuSample>,
}
impl Native {
    pub fn open(gpu: Option<GpuSample>) -> Self {
        let telemetry = panic::catch_unwind(Telemetry::open)
            .map_err(|_| Diagnostic::new(SourceId::IoReport, Problem::Panicked))
            .and_then(|telemetry| {
                telemetry.map_err(|error| {
                    Diagnostic::unavailable(
                        SourceId::IoReport,
                        Operation::Initialize,
                        Cause::of(error.as_ref()),
                    )
                })
            });
        Self {
            telemetry,
            runtime: Runtime::default(),
            smc: SMC::new().map_err(|error| {
                Diagnostic::unavailable(
                    SourceId::Smc,
                    Operation::Initialize,
                    Cause::of(error.as_ref()),
                )
            }),
            hid: HidSensors {
                reader: IOHIDSensors::new().ok(),
                sampled: None,
                cached: Vec::new(),
            },
            smc_sensors: SmcSensors::new(),
            gpu,
        }
    }
    fn runtime(&mut self) -> Result<RuntimeMetrics, Diagnostic> {
        let telemetry = match &mut self.telemetry {
            Ok(telemetry) => telemetry,
            Err(failure) => {
                let failure = failure.clone();
                std::thread::sleep(WINDOW);
                return Err(failure);
            }
        };
        let now = Instant::now();
        if let Some(notice) = self.runtime.waiting(now) {
            let notice = notice.clone();
            std::thread::sleep(WINDOW);
            return Err(notice);
        }
        self.runtime.attempt(now, |reset| {
            if reset {
                *telemetry = Telemetry::open()?;
            }
            telemetry.sample(WINDOW)
        })
    }

    fn sensors(&mut self) -> SensorBatch {
        let SensorBatch {
            mut readings,
            notices,
        } = self
            .smc_sensors
            .sample(self.smc.as_mut().map_err(|notice| &*notice));
        readings.extend(self.hid.sample().iter().cloned());
        SensorBatch {
            readings,
            notices,
        }
    }
}

struct HidSensors {
    reader: Option<IOHIDSensors>,
    sampled: Option<Instant>,
    cached: Vec<Reading>,
}
impl HidSensors {
    fn sample(&mut self) -> &[Reading] {
        if self
            .sampled
            .is_none_or(|time| time.elapsed() >= SENSOR_REFRESH)
        {
            let mut names = BTreeSet::new();
            self.cached = self
                .reader
                .iter()
                .flat_map(IOHIDSensors::get_metrics)
                .filter(|(name, _)| names.insert(name.clone()))
                .filter_map(|(name, celsius)| {
                    let category = category(&name);
                    Reading::new(
                        name,
                        f64::from(celsius),
                        Unit::Celsius,
                        category,
                        SourceId::IoKit,
                    )
                    .ok()
                })
                .collect();
            self.sampled = Some(Instant::now());
        }
        &self.cached
    }
}

impl Source for Native {
    fn sample(
        &mut self,
        snapshot: &mut Snapshot,
    ) {
        let SensorBatch {
            mut readings,
            mut notices,
        } = panic::catch_unwind(AssertUnwindSafe(|| self.sensors())).unwrap_or_else(|_| {
            SensorBatch {
                readings: Vec::new(),
                notices: vec![Diagnostic::new(SourceId::Smc, Problem::Panicked)],
            }
        });
        if readings.is_empty() {
            notices.push(Diagnostic::new(SourceId::Smc, Problem::NoReadings));
        }
        let runtime = self.runtime();
        let metrics = runtime.as_ref().ok();
        if let Some(metrics) = metrics {
            readings.extend(
                [
                    ("CPU", metrics.cpu_watts, SensorCategory::Cpu),
                    ("GPU", metrics.gpu_watts, SensorCategory::Gpu),
                    ("ANE", metrics.ane_watts, SensorCategory::Other),
                    ("DRAM", metrics.dram_watts, SensorCategory::Other),
                ]
                .into_iter()
                .filter_map(|(name, watts, category)| {
                    Reading::new(
                        format!("{name} / IOReport"),
                        watts?,
                        Unit::Watts,
                        category,
                        SourceId::IoReport,
                    )
                    .ok()
                }),
            );
        }
        notices.extend(runtime.as_ref().err().cloned());
        let smc_gpu_power = readings
            .iter()
            .find(|reading| {
                reading.category == SensorCategory::Gpu
                    && reading.source == SourceId::Smc
                    && reading.unit() == Unit::Watts
            })
            .map(Reading::value);
        let gpu = self.gpu.clone().map(|gpu| GpuSample {
            activity_percent: Measurement::validated(
                metrics
                    .and_then(|metrics| metrics.gpu_active_ratio)
                    .map(|ratio| ratio * 100.0),
            ),
            core_mhz: Measurement::validated(metrics.and_then(|metrics| metrics.gpu_mhz)),
            temperature_celsius: Measurement::validated(hottest(&readings, SensorCategory::Gpu)),
            power_watts: Measurement::validated(
                metrics
                    .and_then(|metrics| metrics.gpu_watts)
                    .or(smc_gpu_power),
            ),
            ..gpu
        });
        snapshot.merge(Snapshot {
            cpu_power_watts: Measurement::validated(metrics.and_then(|metrics| metrics.cpu_watts)),
            cpu_clocks_mhz: metrics
                .map(|metrics| CpuClocks {
                    efficiency_mhz: Measurement::validated(metrics.efficiency_mhz),
                    performance_mhz: Measurement::validated(metrics.performance_mhz),
                })
                .into(),
            readings,
            gpu: gpu.into_iter().collect(),
            notices,
            ..Snapshot::default()
        });
    }
}

fn category(label: &str) -> SensorCategory {
    let label = label.to_lowercase();
    if label.contains("gpu") {
        SensorCategory::Gpu
    } else if label.contains("cpu") || label.contains("soc die") {
        SensorCategory::Cpu
    } else {
        SensorCategory::Other
    }
}

#[cfg(test)]
mod tests {
    use std::time::{
        Duration,
        Instant,
    };

    use super::Runtime;
    use crate::model::Problem;
    use crate::source::REDISCOVERY;

    #[test]
    fn runtime_recovers_after_spaced_failures() {
        let now = Instant::now();
        let mut runtime = Runtime::default();
        let first = runtime
            .attempt::<()>(now, |reset| {
                assert!(!reset, "The initial sampler is already open");
                Err(std::io::Error::other("temporary failure").into())
            })
            .expect_err("Initial failure");
        for seconds in [0, 1, 29] {
            let failure = runtime
                .attempt::<()>(now + Duration::from_secs(seconds), |_| {
                    panic!("A waiting runtime must not touch the sampler");
                })
                .expect_err("Retry must wait");
            assert_eq!(failure, first);
        }
        let retry = now + REDISCOVERY;
        assert!(
            runtime
                .attempt::<()>(retry, |reset| {
                    assert!(reset, "Recovery must rebuild IOReport");
                    Err(std::io::Error::other("still unavailable").into())
                })
                .is_err(),
            "Repeated failures remain retryable"
        );
        assert!(
            runtime.waiting(retry + Duration::from_secs(29)).is_some(),
            "A repeated failure starts a new delay"
        );
        assert_eq!(
            runtime.attempt(retry + REDISCOVERY, |reset| {
                assert!(reset, "Retry must reinitialize IOReport");
                Ok(42)
            }),
            Ok(42)
        );
        assert_eq!(
            runtime.attempt(retry + REDISCOVERY, |reset| {
                assert!(!reset, "Recovered runtime resumes ordinary sampling");
                Ok(43)
            }),
            Ok(43)
        );
    }

    #[test]
    fn runtime_panic_stops_all_future_attempts() {
        let now = Instant::now();
        let mut runtime = Runtime::default();
        let notice = runtime
            .attempt::<()>(now, |_| panic!("vendor panic"))
            .expect_err("Panic must be reported");
        assert_eq!(notice.problem, Problem::Panicked);
        let mut called = false;
        let result = runtime.attempt(now + Duration::from_secs(3600), |_| {
            called = true;
            Ok(())
        });
        assert_eq!(result, Err(notice));
        assert!(!called, "A panicked runtime is never reused");
    }
}
