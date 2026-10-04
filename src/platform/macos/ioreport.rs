use std::time::Duration;

use macmon::sources::{
    IOReport,
    IOReportIteratorItem,
    SocInfo,
    WithError,
    cfio_get_residencies,
    cfio_watts,
    get_soc_info,
};

const CPU_STATS: (&str, &str) = ("CPU Stats", "CPU Core Performance States");
const GPU_STATS: (&str, &str) = ("GPU Stats", "GPU Performance States");
const ENERGY: &str = "Energy Model";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RuntimeMetrics {
    pub gpu_active_ratio: Option<f64>,
    pub gpu_mhz: Option<u32>,
    pub efficiency_mhz: Option<u32>,
    pub performance_mhz: Option<u32>,
    pub cpu_watts: Option<f64>,
    pub gpu_watts: Option<f64>,
    pub ane_watts: Option<f64>,
    pub dram_watts: Option<f64>,
}

pub struct Telemetry {
    soc: SocInfo,
    report: IOReport,
}
impl Telemetry {
    pub fn open() -> WithError<Self> {
        let soc = get_soc_info()?;
        let mut report = IOReport::new(vec![
            (ENERGY, None),
            (CPU_STATS.0, Some(CPU_STATS.1)),
            (GPU_STATS.0, Some(GPU_STATS.1)),
        ])?;
        drop(report.try_get_samples(0, 1)?);
        Ok(Self {
            soc,
            report,
        })
    }

    pub fn sample(
        &mut self,
        window: Duration,
    ) -> WithError<RuntimeMetrics> {
        let millis = u64::try_from(window.as_millis()).unwrap_or(u64::MAX);
        let mut metrics = RuntimeMetrics::default();
        let mut efficiency = Cluster::default();
        let mut performance = Cluster::default();
        let mut known = false;
        for (items, elapsed) in self.report.try_get_samples(millis, 1)? {
            let elapsed = Duration::from_millis(elapsed);
            for item in items {
                known |= self.read(
                    &item,
                    elapsed,
                    &mut metrics,
                    &mut efficiency,
                    &mut performance,
                );
            }
        }
        if !known {
            return Err("IOReport returned no known channel".into());
        }
        metrics.efficiency_mhz = efficiency.mhz(&self.soc.ecpu_freqs);
        metrics.performance_mhz = performance.mhz(&self.soc.pcpu_freqs);
        Ok(metrics)
    }

    fn read(
        &self,
        item: &IOReportIteratorItem,
        elapsed: Duration,
        metrics: &mut RuntimeMetrics,
        efficiency: &mut Cluster,
        performance: &mut Cluster,
    ) -> bool {
        let channel = item.channel.as_str();
        match (item.group.as_str(), item.subgroup.as_str()) {
            CPU_STATS => {
                let (cluster, table) = if channel.contains("PCPU") {
                    (performance, &self.soc.pcpu_freqs)
                } else if channel.contains("ECPU") || channel.contains("MCPU") {
                    (efficiency, &self.soc.ecpu_freqs)
                } else {
                    return false;
                };
                cluster.add(residency_clock(&cfio_get_residencies(item.item), table));
            }
            GPU_STATS if channel == "GPUPH" => {
                if let Some(clock) = self
                    .soc
                    .gpu_freqs
                    .get(1..)
                    .and_then(|table| residency_clock(&cfio_get_residencies(item.item), table))
                {
                    metrics.gpu_active_ratio = Some(clock.active_ratio);
                    metrics.gpu_mhz = Some(clock.mhz as u32);
                }
            }
            (ENERGY, _) => {
                let total = if channel == "GPU Energy" {
                    &mut metrics.gpu_watts
                } else if channel.ends_with("CPU Energy") {
                    &mut metrics.cpu_watts
                } else if channel.starts_with("ANE") {
                    &mut metrics.ane_watts
                } else if channel.starts_with("DRAM") {
                    &mut metrics.dram_watts
                } else {
                    return false;
                };
                if let Ok(watts) = cfio_watts(item.item, &item.unit, elapsed) {
                    *total = Some(total.unwrap_or(0.0) + f64::from(watts));
                }
            }
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Clock {
    mhz: f64,
    active_ratio: f64,
}

fn residency_clock(
    residencies: &[(String, i64)],
    table: &[u32],
) -> Option<Clock> {
    if table.is_empty() || table.contains(&0) || residencies.iter().any(|(_, value)| *value < 0) {
        return None;
    }
    let inactive = |name: &str| matches!(name, "IDLE" | "DOWN" | "OFF");
    let offset = residencies.iter().position(|(name, _)| !inactive(name))?;
    let active = residencies.get(offset..)?;
    if active.len() != table.len() || active.iter().any(|(name, _)| inactive(name)) {
        return None;
    }
    let usage: f64 = active.iter().map(|(_, residency)| *residency as f64).sum();
    let total: f64 = residencies
        .iter()
        .map(|(_, residency)| *residency as f64)
        .sum();
    let weighted: f64 = active
        .iter()
        .zip(table)
        .map(|((_, residency), mhz)| *residency as f64 * f64::from(*mhz))
        .sum();
    Some(Clock {
        mhz: if usage > 0.0 { weighted / usage } else { 0.0 },
        active_ratio: if total > 0.0 { usage / total } else { 0.0 },
    })
}

#[derive(Default)]
struct Cluster {
    total_mhz: f64,
    cores: u32,
}
impl Cluster {
    fn add(
        &mut self,
        clock: Option<Clock>,
    ) {
        if let Some(clock) = clock {
            self.total_mhz += clock.mhz;
            self.cores += 1;
        }
    }

    fn mhz(
        &self,
        table: &[u32],
    ) -> Option<u32> {
        let floor = f64::from(table.first().copied().unwrap_or_default());
        (self.cores > 0).then(|| (self.total_mhz / f64::from(self.cores)).max(floor) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Cluster,
        residency_clock,
    };

    fn states(entries: &[(&str, i64)]) -> Vec<(String, i64)> {
        entries
            .iter()
            .map(|(name, residency)| ((*name).to_owned(), *residency))
            .collect()
    }

    #[test]
    fn residency_clock_weights_active_states() {
        let clock = residency_clock(
            &states(&[("IDLE", 50), ("V0", 25), ("V1", 25)]),
            &[1000, 2000],
        )
        .expect("States match the table");
        assert!((clock.mhz - 1500.0).abs() < f64::EPSILON, "Weighted clock");
        assert!(
            (clock.active_ratio - 0.5).abs() < f64::EPSILON,
            "Active ratio"
        );
    }

    #[test]
    fn residency_clock_rejects_inconsistent_states() {
        assert_eq!(residency_clock(&[], &[1000]), None, "No states");
        assert_eq!(
            residency_clock(&states(&[("IDLE", 10), ("OFF", 5)]), &[1000]),
            None,
            "Only idle states"
        );
        assert_eq!(
            residency_clock(
                &states(&[("DOWN", 1), ("IDLE", 1), ("V0", 1)]),
                &[1000, 2000]
            ),
            None,
            "Fewer active states than frequencies"
        );
    }

    #[test]
    fn residency_clock_handles_idle_counters() {
        let clock = residency_clock(&states(&[("IDLE", 0), ("V0", 0)]), &[1000])
            .expect("States match the table");
        assert_eq!(clock.mhz, 0.0, "No active time");
        assert_eq!(clock.active_ratio, 0.0, "No time at all");
    }

    #[test]
    fn residency_clock_rejects_invalid_tables_and_counters() {
        for (entries, table) in [
            (vec![("V0", 1)], vec![]),
            (vec![("V0", 1)], vec![0]),
            (vec![("V0", -1)], vec![1000]),
            (vec![("IDLE", -1), ("V0", 1)], vec![1000]),
            (vec![("V0", 1), ("V1", 1)], vec![1000]),
            (vec![("V0", 1), ("OFF", 1)], vec![1000, 2000]),
            (vec![("V0", 1), ("DOWN", 1)], vec![1000, 2000]),
            (vec![("V0", 1), ("IDLE", 1)], vec![1000, 2000]),
        ] {
            assert_eq!(
                residency_clock(&states(&entries), &table),
                None,
                "Invalid residency layout or frequency table: {entries:?}, {table:?}"
            );
        }
    }

    #[test]
    fn residency_clock_includes_all_inactive_time() {
        let clock = residency_clock(
            &states(&[
                ("DOWN", 60),
                ("IDLE", 20),
                ("OFF", 10),
                ("V0", 10),
                ("V1", 0),
            ]),
            &[1000, 2000],
        )
        .expect("Inactive states precede a complete active table");
        assert_eq!(clock.mhz, 1000.0, "Frequency uses active time only");
        assert!(
            (clock.active_ratio - 0.1).abs() < f64::EPSILON,
            "All inactive time counts"
        );
        let active = residency_clock(&states(&[("V0", 1), ("V1", 1)]), &[1000, 2000])
            .expect("An inactive prefix is optional");
        assert_eq!(active.active_ratio, 1.0, "Entire interval is active");
    }

    #[test]
    fn cluster_clock_is_floored_and_optional() {
        let mut cluster = Cluster::default();
        assert_eq!(cluster.mhz(&[600, 3000]), None, "No cores reported");
        cluster.add(residency_clock(&states(&[("IDLE", 1), ("V0", 0)]), &[600]));
        cluster.add(None);
        assert_eq!(
            cluster.mhz(&[600, 3000]),
            Some(600),
            "Idle cluster at its floor"
        );
    }
}
