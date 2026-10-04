use std::collections::{
    BTreeMap,
    BTreeSet,
    VecDeque,
};
use std::time::Instant;

use super::{
    GpuMetric,
    Reading,
    SensorCategory,
    Snapshot,
    SourceId,
    hottest,
    limits,
};

struct Series<T> {
    samples: VecDeque<(Instant, T)>,
}
impl<T> Series<T> {
    fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(limits::HISTORY_SAMPLES),
        }
    }
    fn push(
        &mut self,
        now: Instant,
        value: T,
    ) {
        self.samples.push_back((now, value));
        self.trim(now);
    }
    fn trim(
        &mut self,
        now: Instant,
    ) {
        while self.samples.len() > limits::HISTORY_SAMPLES
            || self.samples.front().is_some_and(|(time, _)| {
                now.saturating_duration_since(*time).as_secs_f64() > limits::HISTORY_SECONDS
            })
        {
            self.samples.pop_front();
        }
    }
    fn values(&self) -> impl Iterator<Item = &T> { self.samples.iter().map(|(_, value)| value) }
}

#[derive(Clone, Copy, Default)]
struct GpuPoint {
    activity: Option<f64>,
    temperature: Option<f64>,
    power: Option<f64>,
}

struct GpuHistory {
    series: Series<GpuPoint>,
    last_seen: Instant,
}

pub struct History {
    cpu_temperature: Series<Option<f64>>,
    gpu_temperature: Series<Option<f64>>,
    cpu_activity: Series<Option<f64>>,
    gpu: BTreeMap<String, GpuHistory>,
    maxima: BTreeMap<SourceId, BTreeMap<String, f64>>,
}

impl History {
    pub fn new() -> Self {
        Self {
            cpu_temperature: Series::new(),
            gpu_temperature: Series::new(),
            cpu_activity: Series::new(),
            gpu: BTreeMap::new(),
            maxima: BTreeMap::new(),
        }
    }
    pub fn update_snapshot(
        &mut self,
        now: Instant,
        snapshot: &Snapshot,
    ) {
        self.update(now, &snapshot.readings);
        self.cpu_activity.push(now, snapshot.cpu_usage.numeric());
        self.gpu.retain(|_, history| {
            now.saturating_duration_since(history.last_seen)
                .as_secs_f64()
                <= limits::HISTORY_SECONDS
        });
        for (identity, history) in &mut self.gpu {
            if !snapshot.gpu.iter().any(|gpu| &gpu.identity == identity) {
                history.series.push(now, GpuPoint::default());
            }
        }
        for gpu in snapshot.gpu.iter().take(limits::GPUS) {
            if !self.gpu.contains_key(&gpu.identity) && self.gpu.len() >= limits::GPUS {
                continue;
            }
            let history = self
                .gpu
                .entry(gpu.identity.clone())
                .or_insert_with(|| GpuHistory {
                    series: Series::new(),
                    last_seen: now,
                });
            history.last_seen = now;
            history.series.push(
                now,
                GpuPoint {
                    activity: gpu.activity_percent.numeric(),
                    temperature: gpu.temperature_celsius.numeric(),
                    power: gpu.power_watts.numeric(),
                },
            );
        }
    }

    pub fn cpu_activity_series(&self) -> Vec<Option<f64>> {
        self.cpu_activity.values().copied().collect()
    }
    pub fn gpu_series(
        &self,
        identity: &str,
        metric: GpuMetric,
    ) -> Vec<Option<f64>> {
        self.gpu
            .get(identity)
            .into_iter()
            .flat_map(|history| history.series.values())
            .map(|point| match metric {
                GpuMetric::Activity => point.activity,
                GpuMetric::Temperature => point.temperature,
                GpuMetric::Power => point.power,
            })
            .collect()
    }
    pub fn update(
        &mut self,
        now: Instant,
        readings: &[Reading],
    ) {
        self.cpu_temperature
            .push(now, hottest(readings, SensorCategory::Cpu));
        self.gpu_temperature
            .push(now, hottest(readings, SensorCategory::Gpu));
        let readings = readings.get(..limits::READINGS).unwrap_or(readings);
        if self.maxima_len() >= limits::READINGS {
            let active = readings
                .iter()
                .map(|reading| (reading.source, reading.name.as_str()))
                .collect::<BTreeSet<_>>();
            self.maxima.retain(|source, entries| {
                entries.retain(|name, _| active.contains(&(*source, name.as_str())));
                !entries.is_empty()
            });
        }
        for reading in readings {
            if let Some(entry) = self
                .maxima
                .get_mut(&reading.source)
                .and_then(|entries| entries.get_mut(&reading.name))
            {
                *entry = entry.max(reading.value());
            } else if self.maxima_len() < limits::READINGS {
                self.maxima
                    .entry(reading.source)
                    .or_default()
                    .insert(reading.name.clone(), reading.value());
            }
        }
    }
    fn maxima_len(&self) -> usize { self.maxima.values().map(BTreeMap::len).sum() }
    pub fn maximum(
        &self,
        name: &str,
        source: SourceId,
    ) -> Option<f64> {
        self.maxima
            .get(&source)
            .and_then(|entries| entries.get(name))
            .copied()
    }
    pub fn series(
        &self,
        category: SensorCategory,
    ) -> Vec<Option<f64>> {
        match category {
            SensorCategory::Cpu => self.cpu_temperature.values().copied().collect(),
            SensorCategory::Gpu => self.gpu_temperature.values().copied().collect(),
            SensorCategory::System | SensorCategory::Fan | SensorCategory::Other => {
                vec![None; self.cpu_temperature.samples.len()]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{
        Duration,
        Instant,
    };

    use super::History;
    use crate::model::{
        GpuMetric,
        GpuSample,
        Measurement,
        MeasurementError,
        Reading,
        SensorCategory,
        Snapshot,
        SourceId,
        Unit,
    };

    fn celsius(
        name: &str,
        value: f64,
        category: SensorCategory,
        source: SourceId,
    ) -> Result<Reading, MeasurementError> {
        Reading::new(name.into(), value, Unit::Celsius, category, source)
    }
    #[test]
    fn maxima_are_bounded_and_separate_sources() -> Result<(), MeasurementError> {
        let other_source = SourceId::OTHER;
        let readings = [
            celsius("same label", 50.0, SensorCategory::Cpu, SourceId::SENSOR)?,
            celsius("same label", 70.0, SensorCategory::Cpu, other_source)?,
        ];
        let mut history = History::new();
        history.update(Instant::now(), &readings);
        assert_eq!(history.maximum("same label", SourceId::SENSOR), Some(50.0));
        assert_eq!(history.maximum("same label", other_source), Some(70.0));
        for source in [SourceId::SENSOR, other_source] {
            let readings = (0..1100)
                .filter_map(|index| {
                    celsius(&index.to_string(), 50.0, SensorCategory::Cpu, source).ok()
                })
                .collect::<Vec<_>>();
            history.update(Instant::now(), &readings);
            assert_eq!(history.maxima_len(), 1024);
        }
        assert_eq!(history.maximum("same label", SourceId::SENSOR), None);
        Ok(())
    }
    #[test]
    fn cpu_history_keeps_state_gaps_and_real_zero() {
        let mut history = History::new();
        for cpu_usage in [
            Measurement::WarmingUp,
            Measurement::validated(Some(0.0)),
            Measurement::Stale,
        ] {
            history.update_snapshot(
                Instant::now(),
                &Snapshot {
                    cpu_usage,
                    ..Snapshot::default()
                },
            );
        }
        assert_eq!(history.cpu_activity_series(), vec![None, Some(0.0), None]);
    }
    #[test]
    fn gpu_histories_are_bounded_and_keep_identity_and_gaps() {
        let now = Instant::now();
        let snapshot = Snapshot {
            gpu: vec![
                GpuSample {
                    identity: "GPU-A".into(),
                    activity_percent: Measurement::validated(Some(80.0)),
                    ..GpuSample::default()
                },
                GpuSample {
                    identity: "GPU-B".into(),
                    activity_percent: Measurement::validated(Some(20.0)),
                    ..GpuSample::default()
                },
            ],
            ..Snapshot::default()
        };
        let mut history = History::new();
        for index in 0..700 {
            history.update_snapshot(now + Duration::from_millis(index * 100), &snapshot);
        }
        assert_eq!(history.gpu_series("GPU-A", GpuMetric::Activity).len(), 601);
        assert_eq!(
            history.gpu_series("GPU-B", GpuMetric::Activity).last(),
            Some(&Some(20.0))
        );
        history.update_snapshot(now + Duration::from_secs(71), &Snapshot::default());
        history.update_snapshot(now + Duration::from_secs(72), &snapshot);
        assert_eq!(
            history
                .gpu_series("GPU-A", GpuMetric::Activity)
                .iter()
                .rev()
                .nth(1),
            Some(&None)
        );
    }
    #[test]
    fn returning_gpu_keeps_gaps_and_renews_its_lifetime() {
        let now = Instant::now();
        let mut history = History::new();
        let snapshot = Snapshot {
            gpu: vec![GpuSample {
                identity: "returning".into(),
                activity_percent: Measurement::validated(Some(42.0)),
                ..GpuSample::default()
            }],
            ..Snapshot::default()
        };
        history.update_snapshot(now, &snapshot);
        for seconds in 1..120 {
            history.update_snapshot(now + Duration::from_secs(seconds), &Snapshot::default());
        }
        history.update_snapshot(now + Duration::from_secs(120), &snapshot);
        let values = history.gpu_series("returning", GpuMetric::Activity);
        assert_eq!(values.first(), Some(&Some(42.0)));
        assert_eq!(values.get(119), Some(&None));
        assert_eq!(values.last(), Some(&Some(42.0)));
        for seconds in 121..=240 {
            history.update_snapshot(now + Duration::from_secs(seconds), &Snapshot::default());
        }
        assert_eq!(history.gpu.len(), 1);
        history.update_snapshot(now + Duration::from_secs(241), &Snapshot::default());
        assert!(
            history.gpu.is_empty(),
            "Expiry starts at the last actual presence"
        );
    }

    #[test]
    fn absent_gpus_expire_and_release_capacity() {
        let now = Instant::now();
        let mut history = History::new();
        let mut snapshot = Snapshot {
            gpu: (0..crate::model::limits::GPUS)
                .map(|index| GpuSample {
                    identity: index.to_string(),
                    activity_percent: Measurement::validated(Some(42.0)),
                    ..GpuSample::default()
                })
                .collect(),
            ..Snapshot::default()
        };
        history.update_snapshot(now, &snapshot);
        snapshot.gpu = vec![GpuSample {
            identity: "replacement".into(),
            activity_percent: Measurement::validated(Some(7.0)),
            ..GpuSample::default()
        }];
        for seconds in 1..=120 {
            history.update_snapshot(now + Duration::from_secs(seconds), &snapshot);
        }
        assert_eq!(history.gpu.len(), crate::model::limits::GPUS);
        assert!(
            history
                .gpu_series("replacement", GpuMetric::Activity)
                .is_empty(),
            "An unexpired full history cannot admit another identity"
        );
        history.update_snapshot(now + Duration::from_secs(121), &snapshot);
        assert_eq!(history.gpu.len(), 1);
        assert_eq!(
            history.gpu_series("replacement", GpuMetric::Activity),
            vec![Some(7.0)]
        );
        assert!(
            history.gpu_series("0", GpuMetric::Activity).is_empty(),
            "Gap points must not keep an absent GPU alive"
        );
    }

    #[test]
    fn history_is_bounded_and_keeps_session_maximum() {
        let now = Instant::now();
        let mut history = History::new();
        for index in 0..700 {
            history.update(
                now + Duration::from_secs(index),
                &celsius(
                    "CPU",
                    (index % 100) as f64,
                    SensorCategory::Cpu,
                    SourceId::SENSOR,
                )
                .into_iter()
                .collect::<Vec<_>>(),
            );
        }
        assert_eq!(history.series(SensorCategory::Cpu).len(), 121);
        assert_eq!(history.maximum("CPU", SourceId::SENSOR), Some(99.0));
    }
    #[test]
    fn missing_is_not_zero() {
        let mut history = History::new();
        history.update(Instant::now(), &[]);
        history.update(Instant::now(), &[]);
        assert_eq!(history.series(SensorCategory::Cpu), vec![None, None]);
    }
    #[test]
    fn sensor_categories_are_independent_of_display_names() -> Result<(), MeasurementError> {
        let readings = [
            celsius(
                "GPU misleading label",
                50.0,
                SensorCategory::Cpu,
                SourceId::SENSOR,
            )?,
            celsius(
                "CPU misleading label",
                70.0,
                SensorCategory::Gpu,
                SourceId::SENSOR,
            )?,
        ];
        let mut history = History::new();
        history.update(Instant::now(), &readings);
        assert_eq!(history.series(SensorCategory::Cpu), vec![Some(50.0)]);
        assert_eq!(history.series(SensorCategory::Gpu), vec![Some(70.0)]);
        Ok(())
    }
}
