use crossterm::style::Color;

use super::cores::core_rows;
use super::gpu::vram_breakdown;
use super::system::system_header;
use crate::model::{
    MemoryUsage,
    SensorCategory,
};
use crate::ui::primitives::{
    Fill,
    Row,
    clip,
    memory_gib,
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

pub(crate) fn minimal_rows(
    view: &View<'_>,
    width: usize,
) -> Vec<Row> {
    let sample = view.sample;
    let gpu = sample.gpu.iter().enumerate().flat_map(|(index, gpu)| {
        let memory = gpu.memory_bytes.available();
        panel(
            &format!("GPU {index} / {}", gpu.name),
            vec![
                Row::new(
                    format!(
                        "Activity {} · VRAM {}",
                        optional(gpu.activity_percent.numeric(), "%"),
                        memory
                            .map(|memory| format!(
                                "{} / {} occupied",
                                memory_size(memory.occupied()),
                                memory_size(memory.total())
                            ))
                            .unwrap_or_else(|| "N/A".into())
                    ),
                    Color::White,
                ),
                Row::new(
                    memory
                        .map(vram_breakdown)
                        .unwrap_or_else(|| "VRAM N/A".into()),
                    MUTED,
                ),
                Row::new(
                    format!(
                        "Temp {} · power {}/{}",
                        optional(gpu.temperature_celsius.numeric(), "°C"),
                        optional(gpu.power_watts.numeric(), " W"),
                        optional(gpu.power_limit_watts.numeric(), " W")
                    ),
                    MUTED,
                ),
            ],
            width,
            ACCENT,
        )
    });
    let memory = sample.memory_bytes.available();
    let system = panel(
        "CPU / MEMORY",
        vec![
            usage_row(
                "CPU",
                sample.cpu_usage.numeric(),
                format!("peak {}", optional(sample.peak_core(), "%")),
                width.saturating_sub(4),
                Fill::Gradient,
            ),
            usage_row(
                "RAM",
                memory.map(MemoryUsage::used_percent),
                memory_gib(memory),
                width.saturating_sub(4),
                Fill::Gradient,
            ),
            Row::new(
                format!(
                    "CPU temp {} · power {}",
                    optional(sample.hottest(SensorCategory::Cpu), "°C"),
                    optional(sample.cpu_power(), " W")
                ),
                MUTED,
            ),
        ],
        width,
        Color::Cyan,
    );
    let fans = sample
        .fans()
        .take(2)
        .map(|reading| format!("{:.0} RPM", reading.value()))
        .collect::<Vec<_>>()
        .join(" / ");
    system_header(view, width, false)
        .rows
        .into_iter()
        .chain(gpu)
        .chain(system)
        .chain(core_rows(sample, width))
        .chain([Row::new(
            clip(
                &if fans.is_empty() {
                    "Read-only telemetry".into()
                } else {
                    format!("Fans {fans}")
                },
                width,
            ),
            MUTED,
        )])
        .collect()
}
