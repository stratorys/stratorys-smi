mod ioreport;
mod native;
mod smc;

use std::collections::BTreeSet;
use std::time::Duration;

use sysinfo::{
    CpuRefreshKind,
    Disks,
    RefreshKind,
};

use crate::cli::Options;
use crate::model::{
    Bytes,
    GpuSample,
    MemoryTopology,
    Snapshot,
    SourceId,
    StorageInfo,
    SystemInfo,
    limits,
};
use crate::source::{
    Background,
    Source,
};

const RUNTIME_EXPIRY: Duration = Duration::from_secs(3);

pub fn sources(_options: &Options) -> Vec<Box<dyn Source>> {
    let gpu = sysinfo::System::new_with_specifics(
        RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()),
    )
    .cpus()
    .first()
    .map(|cpu| cpu.brand().to_owned())
    .filter(|brand| brand.starts_with("Apple "))
    .map(|brand| GpuSample {
        identity: "Apple integrated GPU".into(),
        name: format!("{brand} GPU"),
        memory_topology: MemoryTopology::Unified,
        ..GpuSample::default()
    });
    let placeholder = Snapshot {
        gpu: gpu.iter().cloned().collect(),
        ..Snapshot::default()
    };
    vec![
        Box::new(Volumes),
        Box::new(Background::spawn(
            SourceId::IoReport,
            Duration::ZERO,
            RUNTIME_EXPIRY,
            placeholder,
            move || native::Native::open(gpu),
        )),
    ]
}

struct Volumes;
impl Source for Volumes {
    fn sample(
        &mut self,
        _snapshot: &mut Snapshot,
    ) {
    }
    fn describe(
        &self,
        info: &mut SystemInfo,
    ) {
        info.storage = Disks::new_with_refreshed_list()
            .iter()
            .take(limits::STORAGE)
            .map(|disk| StorageInfo {
                name: disk.name().to_string_lossy().into_owned(),
                model: None,
                bytes: Bytes(disk.total_space()),
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
}
