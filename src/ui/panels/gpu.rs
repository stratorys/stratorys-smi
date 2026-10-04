use crossterm::style::Color;

use crate::model::{
    GpuMemory,
    GpuMetric,
    GpuSample,
    MemoryTopology,
    MemoryUsage,
};
use crate::ui::charts::graph;
use crate::ui::layout::Layout;
use crate::ui::primitives::{
    Fill,
    Row,
    memory_size,
    optional,
    panel,
    usage_row,
};
use crate::ui::{
    ACCENT,
    MUTED,
    View,
};

const MEMORY: Color = Color::Rgb {
    r: 129,
    g: 140,
    b: 248,
};

fn blank() -> Row { Row::new(String::new(), Color::Reset) }
fn activity_row(
    gpu: &GpuSample,
    width: usize,
) -> Row {
    usage_row(
        "GPU",
        gpu.activity_percent.numeric(),
        format!(
            "{} · {}",
            optional(gpu.temperature_celsius.numeric(), "°C"),
            optional(gpu.power_watts.numeric(), " W")
        ),
        width,
        Fill::Gradient,
    )
}
pub(super) fn vram_breakdown(memory: GpuMemory) -> String {
    format!(
        "allocated {} · reserved {} · free {}",
        memory_size(memory.used()),
        memory_size(memory.reserved()),
        memory_size(memory.free())
    )
}
fn dedicated_memory(
    memory: GpuMemory,
    width: usize,
) -> [Row; 2] {
    [
        usage_row(
            "VRAM",
            Some(memory.occupied_percent()),
            format!(
                "{} / {}",
                memory_size(memory.occupied()),
                memory_size(memory.total())
            ),
            width,
            Fill::Solid(MEMORY),
        ),
        Row::new(vram_breakdown(memory), MUTED),
    ]
}
fn shared_memory(
    memory: Option<MemoryUsage>,
    width: usize,
) -> [Row; 2] {
    [
        memory
            .filter(|memory| memory.total().0 > 0)
            .map(|memory| {
                usage_row(
                    "RAM",
                    Some(memory.used_percent()),
                    format!(
                        "{:.1} / {:.1} GiB",
                        memory.used().gib(),
                        memory.total().gib()
                    ),
                    width,
                    Fill::Solid(MEMORY),
                )
            })
            .unwrap_or_else(|| Row::new("RAM N/A".into(), MUTED)),
        Row::new(
            "Shared system RAM · GPU allocation is not reported separately".into(),
            MUTED,
        ),
    ]
}
fn unified_rows(
    gpu: &GpuSample,
    shared: Option<MemoryUsage>,
    width: usize,
) -> Vec<Row> {
    [
        blank(),
        activity_row(gpu, width),
        blank(),
        Row::new(
            format!(
                "Core {} · activity is active residency",
                optional(gpu.core_mhz.numeric(), " MHz")
            ),
            MUTED,
        ),
        blank(),
    ]
    .into_iter()
    .chain(shared_memory(shared, width))
    .collect()
}
fn sensor_only_rows(gpu: &GpuSample) -> Vec<Row> {
    vec![
        blank(),
        Row::new(
            format!(
                "Temperature {}   ·   power {}",
                optional(gpu.temperature_celsius.numeric(), "°C"),
                optional(gpu.power_watts.numeric(), " W")
            ),
            Color::White,
        ),
        Row::new(
            "Activity and clocks require an additional platform collector.".into(),
            MUTED,
        ),
        blank(),
    ]
}
fn dedicated_rows(
    gpu: &GpuSample,
    width: usize,
    layout: Layout,
) -> Vec<Row> {
    let memory = match gpu.memory_bytes.available() {
        Some(memory) => dedicated_memory(memory, width).to_vec(),
        None => vec![usage_row("VRAM", None, "N/A".into(), width, Fill::Gradient)],
    };
    let limited = gpu
        .clock_limits
        .as_deref()
        .is_some_and(|limits| limits.contains("thermal") || limits.contains("power"));
    let limits = [
        Row::new(
            format!(
                "Power limit {} · slowdown {} · fan {}",
                optional(gpu.power_limit_watts.numeric(), " W"),
                optional(gpu.slowdown_celsius.numeric(), "°C"),
                optional(gpu.fan_percent.numeric(), "%")
            ),
            MUTED,
        ),
        Row::new(
            format!(
                "Clock limits: {}",
                gpu.clock_limits.as_deref().unwrap_or("N/A")
            ),
            if limited { Color::Yellow } else { MUTED },
        ),
    ];
    [blank(), activity_row(gpu, width), blank()]
        .into_iter()
        .chain(memory)
        .chain([
            blank(),
            Row::new(
                format!(
                    "Memory activity {} · core {} · memory {} · {}",
                    optional(gpu.memory_activity_percent.numeric(), "%"),
                    optional(gpu.core_mhz.numeric(), " MHz"),
                    optional(gpu.memory_mhz.numeric(), " MHz"),
                    gpu.performance_state.as_deref().unwrap_or("N/A")
                ),
                MUTED,
            ),
        ])
        .chain(
            limits
                .into_iter()
                .filter(|_| layout.spacious() || layout.details),
        )
        .collect()
}
fn chart_rows(
    view: &View<'_>,
    gpu: &GpuSample,
    width: usize,
    details: bool,
) -> Vec<Row> {
    let activity =
        gpu.memory_topology == MemoryTopology::Unified || gpu.activity_percent.is_available();
    let charts: &[(GpuMetric, &str)] = if details {
        &[
            (GpuMetric::Activity, "Activity (%) · last 120 seconds"),
            (
                GpuMetric::Temperature,
                "Temperature (°C) · last 120 seconds",
            ),
            (GpuMetric::Power, "Power (W) · last 120 seconds"),
        ]
    } else if activity {
        &[(GpuMetric::Activity, "ACTIVITY / LAST 120 SECONDS")]
    } else {
        &[(GpuMetric::Temperature, "TEMPERATURE / LAST 120 SECONDS")]
    };
    charts
        .iter()
        .flat_map(|(metric, title)| {
            std::iter::once(Row::new((*title).into(), MUTED)).chain(
                graph(
                    &view.history.gpu_series(&gpu.identity, *metric),
                    width.saturating_sub(12),
                    3,
                )
                .into_iter()
                .map(|line| Row::new(line, ACCENT)),
            )
        })
        .collect()
}
pub(in crate::ui) fn gpu_panel(
    view: &View<'_>,
    index: usize,
    gpu: &GpuSample,
    width: usize,
    layout: Layout,
) -> Vec<Row> {
    let inner = width.saturating_sub(4);
    let rows = if gpu.memory_topology == MemoryTopology::Unified {
        unified_rows(gpu, view.sample.memory_bytes.available(), inner)
    } else if !gpu.activity_percent.is_available()
        && !gpu.memory_bytes.is_available()
        && !layout.details
    {
        sensor_only_rows(gpu)
    } else {
        dedicated_rows(gpu, inner, layout)
    };
    let charts = if layout.history() {
        chart_rows(view, gpu, width, layout.details)
    } else {
        Vec::new()
    };
    panel(
        &format!("GPU {index} / {}", gpu.name),
        rows.into_iter()
            .filter(|row| layout.spacious() || row.width() > 0)
            .chain(charts)
            .collect(),
        width,
        ACCENT,
    )
}

#[cfg(test)]
mod tests {
    use super::gpu_panel;
    use crate::model::{
        Bytes,
        GpuMemory,
        GpuSample,
        History,
        MemoryTopology,
        Snapshot,
        SystemInfo,
        Uptime,
    };
    use crate::ui::View;
    use crate::ui::layout::{
        Layout,
        LayoutMode,
        Output,
    };
    use crate::ui::panels::minimal::minimal_rows;
    use crate::ui::primitives::{
        memory_size,
        text,
    };

    #[test]
    fn small_allocations_and_driver_reserve_are_visible_in_all_layouts() {
        let mib = 1024 * 1024;
        let gpu = GpuSample {
            memory_bytes: Some(
                GpuMemory::try_from((3 * mib, 86 * mib, 8103 * mib, 8192 * mib))
                    .expect("Valid counters"),
            )
            .into(),
            memory_topology: MemoryTopology::Dedicated,
            ..GpuSample::default()
        };
        let configuration = SystemInfo::default();
        let history = History::new();
        let mut snapshot = Snapshot::default();
        for mode in [LayoutMode::Spacious, LayoutMode::Compact] {
            for output in [Output::Interactive, Output::Plain] {
                let view = View {
                    configuration: &configuration,
                    sample: &snapshot,
                    history: &history,
                    uptime: Uptime::default(),
                };
                let rendered = text(&gpu_panel(
                    &view,
                    0,
                    &gpu,
                    100,
                    Layout {
                        mode,
                        details: false,
                        output,
                    },
                ));
                assert!(
                    rendered.contains("89 MiB / 8.0 GiB"),
                    "Occupied VRAM must include reserve: {rendered}"
                );
                assert!(
                    rendered.contains("1.1%"),
                    "Percentage must include reserve: {rendered}"
                );
                assert!(
                    rendered.contains("allocated 3 MiB · reserved 86 MiB"),
                    "Allocation breakdown must stay visible: {rendered}"
                );
            }
        }
        snapshot.gpu.push(gpu);
        let rendered = text(&minimal_rows(
            &View {
                configuration: &configuration,
                sample: &snapshot,
                history: &history,
                uptime: Uptime::default(),
            },
            100,
        ));
        assert!(
            rendered.contains("89 MiB / 8.0 GiB occupied"),
            "Minimal layout must use occupation: {rendered}"
        );
        assert!(
            rendered.contains("allocated 3 MiB · reserved 86 MiB"),
            "Minimal layout must preserve breakdown: {rendered}"
        );
    }

    #[test]
    fn memory_sizes_do_not_hide_positive_allocations() {
        assert_eq!(memory_size(Bytes(0)), "0 MiB");
        assert_eq!(memory_size(Bytes(1)), "<1 MiB");
        assert_eq!(memory_size(Bytes(1024 * 1024)), "1 MiB");
        assert_eq!(memory_size(Bytes(1024 * 1024 * 1024)), "1.0 GiB");
    }
}
