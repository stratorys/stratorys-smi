use super::panels::layout_rows;
use super::primitives::{
    Row,
    clip,
};
use super::{
    ACCENT,
    MUTED,
    View,
};

pub const WIDTH_MIN: usize = 60;
pub const HEIGHT_MIN: usize = 18;
pub const WIDTH_MAX: usize = 160;
pub const LOGO_WIDTH_MIN: usize = 100;
pub const LOGO_COLUMNS: usize = 30;
pub const PAIR_WIDTH_MIN: usize = 112;
pub fn resize_prompt(
    width: usize,
    height: usize,
    required: usize,
) -> Vec<Row> {
    [
        "STRATORYS / HARDWARE MONITOR".to_owned(),
        format!("Resize to at least {WIDTH_MIN} columns × {required} rows."),
        format!("Current size: {width} × {height} · q exits."),
    ]
    .into_iter()
    .take(height)
    .map(|line| Row::new(clip(&line, width), ACCENT))
    .collect()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LayoutMode {
    Spacious,
    Compact,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Output {
    Interactive,
    Plain,
}
#[derive(Clone, Copy)]
pub(super) struct Layout {
    pub(super) mode: LayoutMode,
    pub(super) details: bool,
    pub(super) output: Output,
}
impl Layout {
    pub(super) fn spacious(self) -> bool { self.mode == LayoutMode::Spacious }
    pub(super) fn logo(self) -> bool { self.output == Output::Interactive }
    pub(super) fn history(self) -> bool { self.spacious() && self.output == Output::Interactive }
}
pub(crate) struct Frame {
    pub rows: Vec<Row>,
    pub logo_rows: usize,
}

pub fn rows(
    view: &View<'_>,
    width: usize,
    details: bool,
    output: Output,
) -> Vec<Row> {
    frame(view, width, details, output).rows
}
pub fn frame(
    view: &View<'_>,
    width: usize,
    details: bool,
    output: Output,
) -> Frame {
    layout_rows(
        view,
        width,
        Layout {
            mode: LayoutMode::Spacious,
            details,
            output,
        },
    )
}
pub fn height_min(minimal: &[Row]) -> usize { HEIGHT_MIN.max(minimal.len() + 1) }

pub fn overview(
    view: &View<'_>,
    width: usize,
    height: usize,
    minimal: Vec<Row>,
) -> Frame {
    let mut frame = [LayoutMode::Spacious, LayoutMode::Compact]
        .into_iter()
        .map(|mode| {
            layout_rows(
                view,
                width,
                Layout {
                    mode,
                    details: false,
                    output: Output::Interactive,
                },
            )
        })
        .find(|frame| frame.rows.len() < height)
        .unwrap_or(Frame {
            rows: minimal,
            logo_rows: 0,
        });
    frame.rows.push(Row::new(
        "q exit · read-only hardware telemetry".into(),
        MUTED,
    ));
    frame
}

pub fn viewport(
    rows: Vec<Row>,
    height: usize,
    page: usize,
) -> Vec<Row> {
    let capacity = height.saturating_sub(1);
    if capacity == 0 {
        return Vec::new();
    }
    let count = rows.len().div_ceil(capacity).max(1);
    let page = page.min(count.saturating_sub(1));
    rows.into_iter()
        .skip(page.saturating_mul(capacity))
        .take(capacity)
        .chain([Row::new(
            format!(
                "DETAILS {}/{count} · ←/→ PgUp/PgDn pages · q exit",
                page + 1
            ),
            MUTED,
        )])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        Layout,
        LayoutMode,
        Output,
        overview,
    };
    use crate::model::{
        GpuMemory,
        GpuSample,
        History,
        Measurement,
        MemoryTopology,
        Reading,
        SensorCategory,
        Snapshot,
        SourceId,
        SystemInfo,
        Unit,
        Uptime,
    };
    use crate::ui::View;
    use crate::ui::panels::{
        layout_rows,
        minimal_rows,
    };

    #[test]
    fn overview_selects_the_largest_fitting_layout() {
        let configuration = SystemInfo::default();
        let history = History::new();
        for width in [60, 99, 100, 111, 112, 160] {
            for topology in [MemoryTopology::Dedicated, MemoryTopology::Unified] {
                for available in [false, true] {
                    for fans in [false, true] {
                        let sample = Snapshot {
                            cores: vec![Measurement::validated(Some(50.0)); 13],
                            gpu: vec![GpuSample {
                                identity: "test".into(),
                                memory_topology: topology,
                                activity_percent: Measurement::validated(available.then_some(0.0)),
                                memory_bytes: available
                                    .then(|| GpuMemory::try_from((0, 0, 10, 10)).ok())
                                    .flatten()
                                    .into(),
                                ..GpuSample::default()
                            }],
                            readings: fans
                                .then(|| {
                                    Reading::new(
                                        "fan".into(),
                                        0.0,
                                        Unit::Rpm,
                                        SensorCategory::Fan,
                                        SourceId::SENSOR,
                                    )
                                })
                                .into_iter()
                                .filter_map(Result::ok)
                                .collect(),
                            ..Snapshot::default()
                        };
                        let view = View {
                            configuration: &configuration,
                            sample: &sample,
                            history: &history,
                            uptime: Uptime::default(),
                        };
                        for mode in [LayoutMode::Spacious, LayoutMode::Compact] {
                            let rendered = layout_rows(
                                &view,
                                width,
                                Layout {
                                    mode,
                                    details: false,
                                    output: Output::Interactive,
                                },
                            );
                            let height = rendered.rows.len() + 1;
                            let frame = overview(&view, width, height, minimal_rows(&view, width));
                            assert!(
                                frame.rows.len() <= height,
                                "The overview must fit {width}/{mode:?}/{topology:?}"
                            );
                            if mode == LayoutMode::Spacious {
                                assert_eq!(
                                    frame.rows.len(),
                                    height,
                                    "The spacious layout must be kept when it fits"
                                );
                            }
                            assert!(
                                overview(&view, width, height - 1, minimal_rows(&view, width))
                                    .rows
                                    .len()
                                    < height,
                                "A one-row reduction must select a smaller layout"
                            );
                        }
                        assert!(
                            overview(&view, width, 0, minimal_rows(&view, width)).logo_rows == 0,
                            "Minimal frames must release logo space"
                        );
                    }
                }
            }
        }
    }
}
