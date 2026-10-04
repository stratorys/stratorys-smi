use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use super::read_bounded;
use crate::model::{
    Bytes,
    HostSample,
    Measurement,
    Percentage,
    Pressure,
    Snapshot,
    StorageInfo,
    SystemInfo,
    TransferRate,
    limits,
};
use crate::source::{
    REDISCOVERY,
    Source,
};

#[derive(Default, Clone, Copy, Debug, PartialEq)]
struct TransferCounters {
    incoming: u64,
    outgoing: u64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct ActivityCounters {
    total: u64,
    wait: u64,
}
struct Counters {
    time: Instant,
    swap: Option<TransferCounters>,
    cpu: Option<ActivityCounters>,
    disks: BTreeMap<String, TransferCounters>,
}

fn pair_delta(
    current: TransferCounters,
    previous: TransferCounters,
    seconds: f64,
) -> Option<TransferRate> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Some(TransferRate {
        incoming: current.incoming.checked_sub(previous.incoming)? as f64 / seconds,
        outgoing: current.outgoing.checked_sub(previous.outgoing)? as f64 / seconds,
    })
}
fn vmstat(text: &str) -> Option<TransferCounters> {
    let field = |name: &str| {
        text.lines().find_map(|line| {
            let mut fields = line.split_whitespace();
            (fields.next() == Some(name))
                .then(|| fields.next()?.parse::<u64>().ok())
                .flatten()
        })
    };
    Some(TransferCounters {
        incoming: field("pswpin")?,
        outgoing: field("pswpout")?,
    })
}
fn cpu_counters(text: &str) -> Option<ActivityCounters> {
    let values = text
        .lines()
        .find(|line| line.starts_with("cpu "))?
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(ActivityCounters {
        total: values
            .iter()
            .try_fold(0u64, |total, value| total.checked_add(*value))?,
        wait: *values.get(4)?,
    })
}
fn disk_counters(
    text: &str,
    names: &[String],
) -> BTreeMap<String, TransferCounters> {
    text.lines()
        .take(512)
        .filter_map(|line| {
            let fields = line.split_whitespace().take(14).collect::<Vec<_>>();
            let name = *fields.get(2)?;
            if !names.iter().any(|candidate| candidate == name) {
                return None;
            }
            Some((
                name.to_owned(),
                TransferCounters {
                    incoming: fields.get(5)?.parse::<u64>().ok()?.checked_mul(512)?,
                    outgoing: fields.get(9)?.parse::<u64>().ok()?.checked_mul(512)?,
                },
            ))
        })
        .collect()
}
fn pressure(text: &str) -> Option<f64> {
    let value = text
        .lines()
        .find(|line| line.starts_with("some "))?
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))?
        .parse::<f64>()
        .ok()?;
    (value.is_finite() && (0.0..=100.0).contains(&value)).then_some(value)
}
fn pressure_of(resource: &str) -> Measurement<Percentage> {
    Measurement::validated(
        read_bounded(format!("/proc/pressure/{resource}"))
            .ok()
            .and_then(|text| pressure(&text)),
    )
}

pub struct Host {
    previous: Option<Counters>,
    disks: Vec<String>,
    discovered: Instant,
}
impl Host {
    pub fn new() -> Self {
        Self {
            previous: None,
            disks: counted_disks(),
            discovered: Instant::now(),
        }
    }
}
impl Source for Host {
    fn sample(
        &mut self,
        snapshot: &mut Snapshot,
    ) {
        if self.discovered.elapsed() >= REDISCOVERY {
            self.disks = counted_disks();
            self.discovered = Instant::now();
        }
        let current = Counters {
            time: Instant::now(),
            swap: read_bounded("/proc/vmstat")
                .ok()
                .and_then(|text| vmstat(&text)),
            cpu: read_bounded("/proc/stat")
                .ok()
                .and_then(|text| cpu_counters(&text)),
            disks: read_bounded("/proc/diskstats")
                .ok()
                .map(|text| disk_counters(&text, &self.disks))
                .unwrap_or_default(),
        };
        let rates = match &self.previous {
            Some(previous) => rates(&current, previous),
            None => HostSample {
                swap_pages_per_second: Measurement::WarmingUp,
                disk_bytes_per_second: Measurement::WarmingUp,
                io_wait_percent: Measurement::WarmingUp,
                ..HostSample::default()
            },
        };
        self.previous = Some(current);
        snapshot.host = Some(HostSample {
            pressure_percent: Pressure {
                cpu: pressure_of("cpu"),
                memory: pressure_of("memory"),
                io: pressure_of("io"),
            },
            ..rates
        });
    }
    fn describe(
        &self,
        info: &mut SystemInfo,
    ) {
        info.storage = storage().into_iter().take(limits::STORAGE).collect();
        info.motherboard = Some(board()).filter(|board| !board.is_empty());
    }
}
fn rates(
    current: &Counters,
    previous: &Counters,
) -> HostSample {
    let seconds = current
        .time
        .saturating_duration_since(previous.time)
        .as_secs_f64();
    HostSample {
        swap_pages_per_second: current
            .swap
            .zip(previous.swap)
            .and_then(|(current, previous)| pair_delta(current, previous, seconds))
            .into(),
        disk_bytes_per_second: current
            .disks
            .iter()
            .filter_map(|(name, current)| pair_delta(*current, *previous.disks.get(name)?, seconds))
            .reduce(|left, right| TransferRate {
                incoming: left.incoming + right.incoming,
                outgoing: left.outgoing + right.outgoing,
            })
            .into(),
        io_wait_percent: Measurement::validated(current.cpu.zip(previous.cpu).and_then(
            |(current, previous)| {
                let total = current.total.checked_sub(previous.total)? as f64;
                let wait = current.wait.checked_sub(previous.wait)? as f64;
                (total > 0.0 && wait <= total).then_some(100.0 * wait / total)
            },
        )),
        ..HostSample::default()
    }
}

struct BlockDevice {
    name: String,
    path: std::path::PathBuf,
}
fn block_devices() -> impl Iterator<Item = BlockDevice> {
    std::fs::read_dir("/sys/block")
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(Result::ok)
        .map(|entry| BlockDevice {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path(),
        })
}
fn counted_disks() -> Vec<String> {
    block_devices()
        .map(|device| device.name)
        .filter(|name| {
            !["loop", "ram", "dm-", "md"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })
        .collect()
}
pub fn storage() -> Vec<StorageInfo> {
    block_devices()
        .filter(|device| device.name.starts_with("sd") || device.name.starts_with("nvme"))
        .filter_map(|device| {
            let sectors = read_bounded(device.path.join("size"))
                .ok()?
                .parse::<u64>()
                .ok()?;
            Some(StorageInfo {
                model: read_bounded(device.path.join("device/model"))
                    .ok()
                    .filter(|model| !model.is_empty()),
                bytes: Bytes(sectors.checked_mul(512)?),
                name: device.name,
            })
        })
        .collect()
}
pub fn board() -> String {
    ["board_vendor", "board_name"]
        .into_iter()
        .filter_map(|field| read_bounded(Path::new("/sys/class/dmi/id").join(field)).ok())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        ActivityCounters,
        Host,
        TransferCounters,
        board,
        cpu_counters,
        disk_counters,
        pair_delta,
        pressure,
        storage,
        vmstat,
    };
    use crate::model::{
        Snapshot,
        TransferRate,
    };
    use crate::source::Source;

    #[test]
    fn missing_counters_fail_gracefully() {
        let mut host = Host::new();
        for _ in 0..2 {
            host.sample(&mut Snapshot::default());
        }
        assert!(host.disks.len() <= 128, "Disk discovery must be bounded");
        let _ = board();
        assert!(
            storage().iter().all(|disk| disk.name.len() <= 255
                && disk.model.as_ref().is_none_or(|model| model.len() <= 65536)
                && disk.bytes.0.is_multiple_of(512)),
            "Disk metadata must remain bounded and use whole sectors"
        );
    }
    #[test]
    fn counters_and_missing_values() {
        let counters = |incoming, outgoing| TransferCounters {
            incoming,
            outgoing,
        };
        assert_eq!(
            pair_delta(counters(20, 40), counters(10, 20), 2.0),
            Some(TransferRate {
                incoming: 5.0,
                outgoing: 10.0
            })
        );
        assert_eq!(pair_delta(counters(1, 40), counters(10, 20), 2.0), None);
        assert_eq!(pair_delta(counters(20, 40), counters(10, 20), 0.0), None);
        assert_eq!(vmstat("pswpin 4\npswpout 8\n"), Some(counters(4, 8)));
        assert_eq!(vmstat("pswpin broken"), None);
        assert_eq!(
            cpu_counters("cpu 1 2 3 4 5 6 7 8 100 100"),
            Some(ActivityCounters {
                total: 36,
                wait: 5
            })
        );
        assert_eq!(pressure("some avg10=1.25 avg60=0.0"), Some(1.25));
        assert_eq!(pressure("some avg10=NaN"), None);
        let disks = disk_counters(
            "8 0 sda 1 0 8 0 1 0 16 0 0 0 0\n8 1 sda1 1 0 8 0 1 0 16 0 0 0 0",
            &["sda".into()],
        );
        assert_eq!(disks.len(), 1);
        assert_eq!(disks.get("sda"), Some(&counters(4096, 8192)));
    }
}
