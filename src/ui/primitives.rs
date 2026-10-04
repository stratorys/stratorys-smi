use crossterm::style::Color;
use unicode_width::{
    UnicodeWidthChar,
    UnicodeWidthStr,
};

use super::MUTED;
use crate::model::{
    Bytes,
    MemoryUsage,
};

#[derive(Clone, PartialEq)]
pub(super) struct Span {
    pub(super) text: String,
    pub(super) color: Color,
}
#[derive(Clone, PartialEq)]
pub struct Row {
    pub(super) spans: Vec<Span>,
}
impl Row {
    pub(super) fn new(
        text: String,
        color: Color,
    ) -> Self {
        Self {
            spans: vec![Span {
                text,
                color,
            }],
        }
    }
    pub(super) fn width(&self) -> usize { self.spans.iter().map(|span| span.text.width()).sum() }
}
pub fn clip(
    text: &str,
    width: usize,
) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .scan(0usize, |used, character| {
            *used = used.saturating_add(character.width().unwrap_or(0));
            (*used <= width).then_some(character)
        })
        .collect()
}
pub(super) fn optional(
    value: Option<f64>,
    unit: &str,
) -> String {
    value
        .map(|value| format!("{value:.1}{unit}"))
        .unwrap_or_else(|| "N/A".into())
}
#[derive(Clone, Copy)]
pub(super) enum Fill {
    Gradient,
    Solid(Color),
}
pub(super) fn usage_row(
    label: &str,
    value: Option<f64>,
    suffix: String,
    width: usize,
    fill: Fill,
) -> Row {
    let size = width.saturating_sub(suffix.width() + 18).clamp(2, 96);
    let filled = value
        .map(|value| ((value.clamp(0.0, 100.0) / 100.0) * size as f64).round() as usize)
        .unwrap_or(0);
    let segments = (0..size).map(|index| Span {
        text: if index < filled { "█" } else { "░" }.into(),
        color: match fill {
            Fill::Gradient | Fill::Solid(_) if index >= filled => Color::DarkGrey,
            Fill::Solid(color) => color,
            Fill::Gradient => {
                let position = index as f64 / size.max(1) as f64;
                Color::Rgb {
                    r: (64.0 + 191.0 * position) as u8,
                    g: (210.0 - 125.0 * position) as u8,
                    b: (180.0 - 120.0 * position) as u8,
                }
            }
        },
    });
    Row {
        spans: std::iter::once(Span {
            text: format!("{label:5} ["),
            color: MUTED,
        })
        .chain(segments)
        .chain([Span {
            text: format!("] {:>6}  {suffix}", optional(value, "%")),
            color: Color::White,
        }])
        .collect(),
    }
}
pub(super) fn panel(
    title: &str,
    content: Vec<Row>,
    width: usize,
    color: Color,
) -> Vec<Row> {
    let inner = width.saturating_sub(4);
    let title = clip(title, width.saturating_sub(6));
    std::iter::once(Row::new(
        format!(
            "╭─ {title} {}╮",
            "─".repeat(width.saturating_sub(title.width() + 5))
        ),
        color,
    ))
    .chain(content.into_iter().map(|row| {
        let used = row.width().min(inner);
        Row {
            spans: std::iter::once(Span {
                text: "│ ".into(),
                color,
            })
            .chain(row.spans.into_iter().scan(inner, |remaining, span| {
                let text = clip(&span.text, *remaining);
                *remaining = remaining.saturating_sub(text.width());
                Some(Span {
                    text,
                    color: span.color,
                })
            }))
            .chain([Span {
                text: format!("{} │", " ".repeat(inner.saturating_sub(used))),
                color,
            }])
            .collect(),
        }
    }))
    .chain([Row::new(
        format!("╰{}╯", "─".repeat(width.saturating_sub(2))),
        color,
    )])
    .collect()
}
pub(super) fn pair(
    left: Vec<Row>,
    right: Vec<Row>,
    column_width: usize,
) -> Vec<Row> {
    let length = left.len().max(right.len());
    let mut left = left.into_iter();
    let mut right = right.into_iter();
    (0..length)
        .map(|_| {
            let left = left
                .next()
                .unwrap_or_else(|| Row::new(String::new(), Color::Reset));
            let right = right
                .next()
                .unwrap_or_else(|| Row::new(String::new(), Color::Reset));
            let padding = column_width.saturating_sub(left.width()) + 2;
            Row {
                spans: left
                    .spans
                    .into_iter()
                    .chain([Span {
                        text: " ".repeat(padding),
                        color: Color::Reset,
                    }])
                    .chain(right.spans)
                    .collect(),
            }
        })
        .collect()
}
pub fn text(rows: &[Row]) -> String {
    rows.iter()
        .map(|row| {
            row.spans
                .iter()
                .map(|span| clip(&span.text, 4096))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn gib(bytes: Option<Bytes>) -> String {
    bytes
        .map(|bytes| format!("{:.1}", bytes.gib()))
        .unwrap_or_else(|| "N/A".into())
}
pub(super) fn memory_gib(memory: Option<MemoryUsage>) -> String {
    format!(
        "{}/{} GiB",
        gib(memory.map(MemoryUsage::used)),
        gib(memory.map(MemoryUsage::total))
    )
}

pub(super) fn notice_rows(
    notices: &[crate::model::Diagnostic],
    details: bool,
    width: usize,
) -> Vec<Row> {
    if !details && notices.is_empty() {
        return vec![Row::new(String::new(), Color::Yellow)];
    }
    notices
        .iter()
        .take(if details { notices.len() } else { 1 })
        .map(|notice| Row::new(clip(&notice.to_string(), width), Color::Yellow))
        .collect()
}

pub(super) fn memory_size(Bytes(bytes): Bytes) -> String {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    match bytes {
        0 => "0 MiB".into(),
        1..MIB => "<1 MiB".into(),
        MIB..GIB => format!("{:.0} MiB", bytes as f64 / MIB as f64),
        _ => format!("{:.1} GiB", bytes as f64 / GIB as f64),
    }
}
