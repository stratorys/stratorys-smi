use crossterm::style::Color;
use unicode_width::UnicodeWidthStr;

use crate::model::{
    MemoryUsage,
    SystemInfo,
    Uptime,
};
use crate::ui::layout::{
    Frame,
    LOGO_COLUMNS,
    LOGO_WIDTH_MIN,
};
use crate::ui::primitives::{
    Row,
    Span,
    clip,
    gib,
    panel,
};
use crate::ui::{
    ACCENT,
    View,
};

fn format_uptime(uptime: &Uptime) -> String {
    let host = uptime
        .host
        .map(|duration| {
            let seconds = duration.as_secs();
            format!(
                "{}d {:02}h {:02}m",
                seconds / 86400,
                (seconds / 3600) % 24,
                (seconds / 60) % 60
            )
        })
        .unwrap_or_else(|| "N/A".into());
    format!("{host} · session {:.0}s", uptime.session.as_secs_f64())
}
pub(super) fn hardware_lines(info: &SystemInfo) -> Vec<String> {
    [
        format!(
            "{} · {}",
            info.hostname.as_deref().unwrap_or("Unknown host"),
            info.os.as_deref().unwrap_or("Unknown OS")
        ),
        format!(
            "Kernel {} · {}",
            info.kernel.as_deref().unwrap_or("N/A"),
            info.architecture
        ),
        format!(
            "CPU {} · {} cores / {} threads",
            info.cpu.as_deref().unwrap_or("unavailable"),
            info.cores_physical
                .map(|count| count.to_string())
                .unwrap_or_else(|| "N/A".into()),
            info.threads
        ),
        format!("RAM {:.1} GiB", info.memory_bytes.gib()),
    ]
    .into_iter()
    .chain(
        info.motherboard
            .iter()
            .map(|board| format!("Motherboard {board}")),
    )
    .chain(info.storage.iter().map(|disk| match &disk.model {
        Some(model) => format!("Disk {}: {model} · {:.0} GB", disk.name, disk.bytes.gb()),
        None => format!("Volume {} · {:.0} GB", disk.name, disk.bytes.gb()),
    }))
    .take(64)
    .collect()
}

pub(in crate::ui) fn system_header(
    view: &View<'_>,
    width: usize,
    logo: bool,
) -> Frame {
    let View {
        configuration,
        sample,
        uptime,
        ..
    } = view;
    let graphic_space = if logo && width >= LOGO_WIDTH_MIN {
        LOGO_COLUMNS
    } else {
        0
    };
    let memory = sample.memory_bytes.available();
    let header_values = [
        ("stratorys-smi", format!("v{}", env!("CARGO_PKG_VERSION"))),
        (
            "Hostname",
            configuration
                .hostname
                .clone()
                .unwrap_or_else(|| "Unknown host".into()),
        ),
        (
            "OS",
            configuration
                .os
                .clone()
                .unwrap_or_else(|| "Unknown OS".into()),
        ),
        (
            "Kernel",
            format!(
                "{} · {}",
                configuration.kernel.as_deref().unwrap_or("N/A"),
                configuration.architecture
            ),
        ),
        (
            "CPU",
            configuration
                .cpu
                .clone()
                .unwrap_or_else(|| "unavailable".into()),
        ),
        ("Cores", sample.cores.len().to_string()),
        (
            "Memory",
            format!(
                "{} / {} GiB",
                gib(memory.map(MemoryUsage::used)),
                gib(memory.map(MemoryUsage::total))
            ),
        ),
        ("Uptime", format_uptime(uptime)),
        (
            "Load",
            sample
                .load_average
                .map(|load| format!("{:.2} / {:.2} / {:.2}", load.one, load.five, load.fifteen))
                .unwrap_or_else(|| "N/A".into()),
        ),
        (
            "Devices",
            if sample.gpu.is_empty() {
                "GPU telemetry unavailable".into()
            } else {
                sample.gpu.len().to_string()
            },
        ),
    ];
    let card_width = width.saturating_sub(graphic_space);
    let available = card_width.saturating_sub(4);
    let content = header_values
        .into_iter()
        .map(|(label, value)| {
            let label = clip(label, available.saturating_sub(3));
            let value = clip(&value, available.saturating_sub(label.width() + 3));
            let leaders = available.saturating_sub(label.width() + value.width() + 2);
            Row {
                spans: vec![
                    Span {
                        text: label,
                        color: ACCENT,
                    },
                    Span {
                        text: format!(" {} ", "─".repeat(leaders)),
                        color: Color::DarkGrey,
                    },
                    Span {
                        text: value,
                        color: Color::White,
                    },
                ],
            }
        })
        .collect();
    let rows: Vec<Row> = panel("SYSTEM", content, card_width, ACCENT)
        .into_iter()
        .map(|row| Row {
            spans: std::iter::once(Span {
                text: " ".repeat(graphic_space),
                color: Color::Reset,
            })
            .chain(row.spans)
            .collect(),
        })
        .collect();
    Frame {
        logo_rows: if graphic_space > 0 { rows.len() } else { 0 },
        rows,
    }
}
