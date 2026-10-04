use std::time::{
    Duration,
    Instant,
};

use sysinfo::{
    CpuRefreshKind,
    MemoryRefreshKind,
    RefreshKind,
};

use crate::model::{
    AvailableKind,
    AvailableMemory,
    Bytes,
    LoadAverage,
    Measurement,
    MemoryUsage,
    Snapshot,
    SystemInfo,
    limits,
};
use crate::source::{
    REDISCOVERY,
    Source,
};

pub struct System {
    system: sysinfo::System,
    discovered: Instant,
}
impl System {
    pub fn new() -> Self {
        Self {
            system: sysinfo::System::new_with_specifics(
                RefreshKind::nothing()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything()),
            ),
            discovered: Instant::now(),
        }
    }
}
impl Source for System {
    fn sample(
        &mut self,
        snapshot: &mut Snapshot,
    ) {
        if self.discovered.elapsed() >= REDISCOVERY {
            self.system.refresh_cpu_list(CpuRefreshKind::everything());
            self.discovered = Instant::now();
        }
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        let system = &self.system;
        let load = sysinfo::System::load_average();
        let kind = if cfg!(target_os = "macos") {
            AvailableKind::Noncompressed
        } else {
            AvailableKind::Available
        };
        snapshot.merge(Snapshot {
            cpu_usage: Measurement::validated(Some(f64::from(system.global_cpu_usage()))),
            load_average: Some(LoadAverage {
                one: load.one,
                five: load.five,
                fifteen: load.fifteen,
            }),
            uptime: Some(Duration::from_secs(sysinfo::System::uptime())),
            cores: system
                .cpus()
                .iter()
                .take(limits::CORES)
                .map(|cpu| Measurement::validated(Some(f64::from(cpu.cpu_usage()))))
                .collect(),
            memory_bytes: MemoryUsage::new(system.used_memory(), system.total_memory())
                .ok()
                .filter(|memory| memory.total() > Bytes(0))
                .into(),
            memory_available: (system.available_memory() <= system.total_memory())
                .then(|| AvailableMemory {
                    bytes: Bytes(system.available_memory()),
                    kind,
                })
                .into(),
            swap_bytes: MemoryUsage::new(system.used_swap(), system.total_swap())
                .ok()
                .into(),
            ..Snapshot::default()
        });
    }
    fn describe(
        &self,
        info: &mut SystemInfo,
    ) {
        let cpus = self.system.cpus();
        info.hostname = sysinfo::System::host_name();
        info.os = sysinfo::System::long_os_version();
        info.kernel = sysinfo::System::kernel_version();
        info.architecture = std::env::consts::ARCH.into();
        info.cpu = cpus.first().map(|cpu| cpu.brand().to_owned());
        info.cores_physical = sysinfo::System::physical_core_count();
        info.threads = cpus.len();
        info.memory_bytes = Bytes(self.system.total_memory());
    }
}
