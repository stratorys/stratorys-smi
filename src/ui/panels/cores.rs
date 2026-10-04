use crossterm::style::Color;

use crate::model::Snapshot;
use crate::ui::primitives::{
    Row,
    Span,
    optional,
};

const CELL: usize = 18;
const COLUMNS_MAX: usize = 8;

pub(in crate::ui) fn core_rows(
    sample: &Snapshot,
    width: usize,
) -> Vec<Row> {
    let columns = width
        .saturating_sub(4)
        .div_euclid(CELL)
        .clamp(1, COLUMNS_MAX);
    sample
        .cores
        .chunks(columns)
        .enumerate()
        .map(|(row, cores)| Row {
            spans: cores
                .iter()
                .enumerate()
                .map(|(column, value)| {
                    let index = row * columns + column;
                    let available = value.numeric();
                    let filled = available
                        .map(|value| (value / 25.0).round() as usize)
                        .unwrap_or(0);
                    Span {
                        text: format!(
                            "C{index:02} {}{} {:>6}  ",
                            "█".repeat(filled.min(4)),
                            "░".repeat(4usize.saturating_sub(filled)),
                            optional(available, "%")
                        ),
                        color: if available.is_some_and(|value| value >= 90.0) {
                            Color::Yellow
                        } else {
                            Color::Cyan
                        },
                    }
                })
                .collect(),
        })
        .collect()
}
