use crossterm::style::Color;

use super::cores::core_rows;
use super::gpu::gpu_panel;
use super::system::{
    hardware_lines,
    system_header,
};
use crate::model::{
    AvailableKind,
    AvailableMemory,
    HostSample,
    MemoryUsage,
    SensorCategory,
    Severity,
    TransferRate,
};
use crate::ui::charts::graph;
use crate::ui::layout::{
    Layout,
    PAIR_WIDTH_MIN,
    WIDTH_MAX,
};
use crate::ui::primitives::{
    Fill,
    Row,
    gib,
    memory_gib,
    notice_rows,
    optional,
    pair,
    panel,
    usage_row,
};
use crate::ui::{
    MUTED,
    View,
};

fn cpu_panel(
    view: &View<'_>,
    width: usize,
) -> Vec<Row> {
    let sample = view.sample;
    let load = match &sample.host {
        Some(host) => format!("I/O wait {}", optional(host.io_wait_percent.numeric(), "%")),
        None => format!(
            "CPU power {} · system power {}",
            optional(sample.cpu_power(), " W"),
            optional(sample.sensor_power(SensorCategory::System), " W")
        ),
    };
    let clocks = sample
        .cpu_clocks_mhz
        .available()
        .map(|clocks| {
            format!(
                "E-cluster {} MHz · P-cluster {} MHz",
                clocks.efficiency_mhz, clocks.performance_mhz
            )
        })
        .unwrap_or_else(|| "CPU clocks N/A".into());
    panel(
        "CPU",
        vec![
            usage_row(
                "CPU",
                sample.cpu_usage.numeric(),
                format!("{} logical cores", sample.cores.len()),
                width.saturating_sub(4),
                Fill::Gradient,
            ),
            Row::new(
                format!(
                    "Hottest CPU {} · peak core {}",
                    optional(sample.hottest(SensorCategory::Cpu), "°C"),
                    optional(sample.peak_core(), "%"),
                ),
                MUTED,
            ),
            Row::new(load, MUTED),
            Row::new(clocks, MUTED),
        ],
        width,
        Color::Cyan,
    )
}
fn memory_panel(
    view: &View<'_>,
    width: usize,
) -> Vec<Row> {
    let sample = view.sample;
    let meter = |label, memory: Option<MemoryUsage>| {
        usage_row(
            label,
            memory.map(MemoryUsage::used_percent),
            memory_gib(memory),
            width.saturating_sub(4),
            Fill::Gradient,
        )
    };
    panel(
        "MEMORY",
        vec![
            meter("RAM", sample.memory_bytes.available()),
            meter("SWAP", sample.swap_bytes.available()),
            Row::new(available_memory(sample.memory_available.available()), MUTED),
        ],
        width,
        Color::Blue,
    )
}
fn available_memory(memory: Option<AvailableMemory>) -> String {
    let label = match memory.map(|memory| memory.kind) {
        Some(AvailableKind::Noncompressed) => "Noncompressed memory",
        Some(AvailableKind::Available) | None => "Available memory",
    };
    format!("{label} {} GiB", gib(memory.map(|memory| memory.bytes)))
}
fn host_panel(
    host: &HostSample,
    width: usize,
) -> Vec<Row> {
    let pressure = host.pressure_percent;
    let rate = |rate: Option<TransferRate>, scale: f64, unit| {
        (
            optional(rate.map(|rate| rate.incoming / scale), unit),
            optional(rate.map(|rate| rate.outgoing / scale), unit),
        )
    };
    let (read, write) = rate(
        host.disk_bytes_per_second.available(),
        1_048_576.0,
        " MiB/s",
    );
    let (swap_in, swap_out) = rate(host.swap_pages_per_second.available(), 1.0, " pages/s");
    panel(
        "HOST PRESSURE / I/O",
        vec![
            Row::new(
                format!(
                    "PSI some avg10: CPU {} · RAM {} · I/O {}",
                    optional(pressure.cpu.numeric(), "%"),
                    optional(pressure.memory.numeric(), "%"),
                    optional(pressure.io.numeric(), "%")
                ),
                MUTED,
            ),
            Row::new(format!("Disk read {read} · write {write}"), MUTED),
            Row::new(format!("Swap in {swap_in} · out {swap_out}"), MUTED),
        ],
        width,
        Color::Cyan,
    )
}
fn fans_panel(
    view: &View<'_>,
    width: usize,
) -> Vec<Row> {
    let fans = view
        .sample
        .fans()
        .take(8)
        .map(|reading| format!("{}: {:.0} RPM", reading.name, reading.value()))
        .collect::<Vec<_>>();
    if fans.is_empty() {
        return Vec::new();
    }
    panel(
        "FANS",
        vec![Row::new(fans.join("   ·   "), Color::Green)],
        width,
        Color::Green,
    )
}
fn history_panel(
    title: &str,
    values: &[Option<f64>],
    height: usize,
    width: usize,
) -> Vec<Row> {
    panel(
        title,
        graph(values, width.saturating_sub(12), height)
            .into_iter()
            .map(|line| Row::new(line, Color::Cyan))
            .collect(),
        width,
        Color::Cyan,
    )
}
fn detail_panels(
    view: &View<'_>,
    width: usize,
    layout: Layout,
) -> Vec<Row> {
    let cores = view
        .sample
        .cores
        .chunks(8)
        .enumerate()
        .map(|(row, cores)| {
            Row::new(
                cores
                    .iter()
                    .enumerate()
                    .map(|(index, usage)| {
                        format!(
                            "C{:02} {:>6}",
                            row * 8 + index,
                            optional(usage.numeric(), "%")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("  "),
                MUTED,
            )
        })
        .collect();
    let sensors = view
        .sample
        .readings
        .iter()
        .map(|reading| {
            Row::new(
                format!(
                    "{}: {:.1} {} · session max {}{}",
                    reading.name,
                    reading.value(),
                    reading.unit().label(),
                    optional(
                        view.history.maximum(&reading.name, reading.source),
                        reading.unit().label()
                    ),
                    if reading.alarm {
                        " · HARDWARE ALARM"
                    } else {
                        ""
                    }
                ),
                match reading.severity() {
                    Severity::Normal => Color::White,
                    Severity::Warning => Color::Yellow,
                    Severity::Critical => Color::Red,
                },
            )
        })
        .collect();
    let hardware = hardware_lines(view.configuration)
        .into_iter()
        .map(|line| Row::new(line, MUTED))
        .collect();
    panel("CPU CORES", cores, width, Color::Cyan)
        .into_iter()
        .chain(panel("SENSORS", sensors, width, Color::Green))
        .chain(panel("HARDWARE", hardware, width, MUTED))
        .chain(if layout.history() {
            history_panel(
                "CPU TEMPERATURE / LAST 120 SECONDS",
                &view.history.series(SensorCategory::Cpu),
                3,
                width,
            )
        } else {
            Vec::new()
        })
        .collect()
}

pub(in crate::ui) fn layout_rows(
    view: &View<'_>,
    width: usize,
    layout: Layout,
) -> crate::ui::layout::Frame {
    let width = width.clamp(8, WIDTH_MAX);
    let spacer = || {
        std::iter::once(Row::new(String::new(), Color::Reset)).take(usize::from(layout.spacious()))
    };
    let gpu = view
        .sample
        .gpu
        .iter()
        .enumerate()
        .flat_map(|(index, gpu)| gpu_panel(view, index, gpu, width, layout));
    let devices = if width >= PAIR_WIDTH_MIN {
        let card_width = width.saturating_sub(2) / 2;
        pair(
            cpu_panel(view, card_width),
            memory_panel(view, card_width),
            card_width,
        )
    } else {
        cpu_panel(view, width)
            .into_iter()
            .chain(memory_panel(view, width))
            .collect()
    };
    let header = system_header(view, width, layout.logo());
    let rows = header
        .rows
        .into_iter()
        .chain(spacer())
        .chain(gpu)
        .chain(spacer())
        .chain(devices)
        .chain(panel(
            "CPU CORES / USAGE",
            core_rows(view.sample, width),
            width,
            Color::Cyan,
        ))
        .chain(if layout.history() {
            history_panel(
                "CPU ACTIVITY / LAST 120 SECONDS",
                &view.history.cpu_activity_series(),
                4,
                width,
            )
        } else {
            Vec::new()
        })
        .chain(
            view.sample
                .host
                .iter()
                .flat_map(|host| host_panel(host, width)),
        )
        .chain(fans_panel(view, width))
        .chain(if layout.details {
            detail_panels(view, width, layout)
        } else {
            Vec::new()
        })
        .chain(notice_rows(&view.sample.notices, layout.details, width))
        .collect();
    crate::ui::layout::Frame {
        rows,
        logo_rows: header.logo_rows,
    }
}
