use unicode_width::UnicodeWidthStr;

use super::charts::graph;
use super::layout::{
    Output,
    height_min,
    overview,
    resize_prompt,
    rows,
    viewport,
};
use super::logo::{
    ImageProtocol,
    LogoLayout,
    image_protocol,
    logo_layout,
};
use super::primitives::{
    Row,
    clip,
    pair,
    text,
};
use super::{
    View,
    minimal_rows,
};
use crate::model::{
    History,
    Measurement,
    Snapshot,
    SystemInfo,
    Uptime,
};

fn view<'view>(
    configuration: &'view SystemInfo,
    sample: &'view Snapshot,
    history: &'view History,
) -> View<'view> {
    View {
        configuration,
        sample,
        history,
        uptime: Uptime::default(),
    }
}
fn is_graphic_header(row: &Row) -> bool {
    row.spans
        .first()
        .is_some_and(|span| span.text == " ".repeat(30))
}
#[test]
fn graphics_detection_avoids_approximated_logos() {
    assert!(
        image_protocol("ghostty", "xterm-256color", false) == ImageProtocol::Kitty,
        "Ghostty must use its native image protocol"
    );
    assert!(
        image_protocol("iTerm.app", "xterm-256color", false) == ImageProtocol::Iterm,
        "iTerm must use inline PNGs"
    );
    assert!(
        image_protocol("ghostty", "xterm-256color", true) == ImageProtocol::None,
        "Multiplexers must use the wordmark fallback"
    );
}
#[test]
fn logo_uses_cell_dimensions_without_stretching() {
    let size = crossterm::terminal::WindowSize {
        columns: 100,
        rows: 100,
        width: 900,
        height: 2100,
    };
    assert!(
        logo_layout(Some(&size))
            == LogoLayout {
                columns: 24,
                left: 0,
                top: 0,
                pixel_offset: 18
            },
        "A square must be centered using physical cell dimensions"
    );
    let wide = crossterm::terminal::WindowSize {
        columns: 100,
        rows: 100,
        width: 2000,
        height: 1000,
    };
    assert!(
        logo_layout(Some(&wide))
            == LogoLayout {
                columns: 6,
                left: 9,
                top: 0,
                pixel_offset: 0
            },
        "Wide character cells must not push the logo outside the header"
    );
    assert!(
        logo_layout(None)
            == LogoLayout {
                columns: 24,
                left: 0,
                top: 0,
                pixel_offset: 0
            },
        "Unavailable pixel dimensions need a bounded fallback"
    );
}
#[test]
fn apple_memory_and_layout_are_present_before_runtime_warmup() {
    let snapshot = |activity| Snapshot {
        gpu: vec![crate::model::GpuSample {
            identity: "Apple integrated GPU".into(),
            name: "Apple GPU".into(),
            memory_topology: crate::model::MemoryTopology::Unified,
            activity_percent: Measurement::validated(activity),
            ..crate::model::GpuSample::default()
        }],
        memory_bytes: crate::model::MemoryUsage::new(16 * 1073741824, 24 * 1073741824)
            .ok()
            .into(),
        ..Snapshot::default()
    };
    let initial = snapshot(None);
    let ready = snapshot(Some(80.0));
    let initial_rows = rows(
        &view(&SystemInfo::default(), &initial, &History::new()),
        120,
        false,
        Output::Interactive,
    );
    let mut ready_history = History::new();
    ready_history.update_snapshot(std::time::Instant::now(), &ready);
    let ready_rows = rows(
        &view(&SystemInfo::default(), &ready, &ready_history),
        120,
        false,
        Output::Interactive,
    );
    assert!(
        text(&initial_rows).contains("Shared system RAM"),
        "Memory must exist on the first frame"
    );
    assert_eq!(
        initial_rows.len(),
        ready_rows.len(),
        "Runtime warmup must not change the layout height"
    );
    assert!(
        !text(&initial_rows).contains("VRAM"),
        "Shared RAM must not be presented as dedicated VRAM"
    );
}
#[test]
fn split_panes_keep_the_header_and_metric_cards() {
    let sample = Snapshot {
        cores: vec![Measurement::validated(Some(20.0)); 12],
        gpu: vec![crate::model::GpuSample {
            identity: "Apple integrated GPU".into(),
            name: "Apple GPU".into(),
            memory_topology: crate::model::MemoryTopology::Unified,
            activity_percent: Measurement::validated(Some(40.0)),
            ..crate::model::GpuSample::default()
        }],
        ..Snapshot::default()
    };
    for width in [120, 160] {
        let visible = overview(
            &view(&SystemInfo::default(), &sample, &History::new()),
            width,
            40,
            minimal_rows(
                &view(&SystemInfo::default(), &sample, &History::new()),
                width,
            ),
        )
        .rows;
        assert!(
            visible.first().is_some_and(is_graphic_header),
            "Split panes must preserve the branded header when it fits"
        );
        assert!(
            text(&visible).contains("CPU CORES / USAGE")
                && text(&visible).contains("Shared system RAM"),
            "Split panes must retain the core and memory cards"
        );
        assert!(
            visible.len() <= 40,
            "The dashboard must still fit without scrolling"
        );
    }
}
#[test]
fn plain_output_aligns_the_system_card_with_metric_cards() {
    for details in [false, true] {
        for width in [60, 120, 160] {
            let visible = rows(
                &view(
                    &SystemInfo::default(),
                    &Snapshot::default(),
                    &History::new(),
                ),
                width,
                details,
                Output::Plain,
            );
            let plain = text(&visible);
            assert!(plain.starts_with("╭─ SYSTEM "));
            assert!(!visible.first().is_some_and(is_graphic_header));
            assert!(visible.iter().take(12).all(|row| row.width() == width));
            assert!(plain.lines().any(|line| line.starts_with("│ Hostname ")));
        }
    }
}
#[test]
fn short_panes_keep_the_system_table_without_the_logo() {
    let sample = Snapshot {
        cores: vec![Measurement::validated(Some(20.0)); 12],
        gpu: vec![crate::model::GpuSample {
            identity: "Apple integrated GPU".into(),
            name: "Apple GPU".into(),
            memory_topology: crate::model::MemoryTopology::Unified,
            ..crate::model::GpuSample::default()
        }],
        ..Snapshot::default()
    };
    let configuration = SystemInfo {
        hostname: Some("host".into()),
        os: Some("macOS".into()),
        kernel: Some("25.6".into()),
        architecture: "aarch64".into(),
        cpu: Some("Apple M4 Pro".into()),
        ..SystemInfo::default()
    };
    for width in [60, 120, 160] {
        let history = History::new();
        let view = view(&configuration, &sample, &history);
        let height = height_min(&minimal_rows(&view, width));
        let visible = overview(&view, width, height, minimal_rows(&view, width)).rows;
        let plain = text(&visible);
        assert!(plain.starts_with("╭─ SYSTEM "));
        assert!(visible.iter().all(|row| row.width() <= width));
        for label in [
            "Hostname", "OS", "Kernel", "CPU", "Cores", "Memory", "Uptime", "Load", "Devices",
        ] {
            assert!(
                plain
                    .lines()
                    .any(|line| line.starts_with(&format!("│ {label} ")) && line.contains('─')),
                "The compact system table must retain {label} with aligned leaders"
            );
        }
        assert!(
            !visible.first().is_some_and(is_graphic_header),
            "Short panes must release logo space"
        );
        assert!(
            visible.len() <= height,
            "The complete table and metrics must fit without scrolling"
        );
    }
}
#[test]
fn overview_fits_without_scrolling() {
    for gpus in [0, 1, 2, 8, 32] {
        for cores in [0, 12, 48, 256] {
            let sample = Snapshot {
                cores: (0..cores)
                    .map(|_| Measurement::validated(Some(100.0)))
                    .collect(),
                gpu: (0..gpus)
                    .map(|index| crate::model::GpuSample {
                        identity: index.to_string(),
                        name: format!("Device {index}"),
                        ..crate::model::GpuSample::default()
                    })
                    .collect(),
                ..Snapshot::default()
            };
            for width in [60, 100, 120, 160] {
                let configuration = SystemInfo::default();
                let history = History::new();
                let view = view(&configuration, &sample, &history);
                let height = height_min(&minimal_rows(&view, width));
                let visible = overview(&view, width, height, minimal_rows(&view, width)).rows;
                assert!(
                    visible.len() <= height,
                    "Overview must fit for {gpus} GPUs at {width} columns"
                );
                assert!(
                    text(&visible).lines().all(|line| line.width() <= width),
                    "Overview must fit width"
                );
                if cores > 0 {
                    assert!(
                        text(&visible).contains(&format!("C{:02}", cores - 1)),
                        "Overview must include every core"
                    );
                }
            }
        }
    }
}
#[test]
fn minimum_prompt_and_zero_height_are_safe() {
    let prompt = resize_prompt(59, 17, 18);
    assert!(
        text(&prompt).contains("60 columns"),
        "Minimum width must be explicit"
    );
    assert!(
        text(&prompt).lines().all(|line| line.width() <= 59),
        "Prompt must fit"
    );
    assert!(
        resize_prompt(0, 0, 18).is_empty(),
        "Zero-height prompt must be empty"
    );
    assert!(
        viewport(Vec::new(), 0, usize::MAX).is_empty(),
        "Zero-height viewport must be empty"
    );
}
#[test]
fn clips_controls_and_wide_characters() {
    assert_eq!(clip("a界\nxyz", 3), "a界");
}
#[test]
fn missing_graph_is_explicit() {
    assert_eq!(graph(&[None], 10, 4), vec!["N/A — no samples", "", "", ""]);
}
#[test]
fn panels_fit_and_scroll() {
    for width in [8, 25, 80, 120, 160] {
        let rows = rows(
            &view(
                &SystemInfo::default(),
                &Snapshot::default(),
                &History::new(),
            ),
            width,
            true,
            Output::Interactive,
        );
        assert!(
            text(&rows).lines().all(|line| line.width() <= width),
            "Rows must fit {width}"
        );
        let pages = rows.len().div_ceil(4);
        let visible = viewport(rows, 5, usize::MAX);
        assert!(visible.len() <= 5, "Detail pages must fit");
        assert!(
            text(&visible).contains(&format!("/{pages}")),
            "Footer must show total"
        );
        assert!(
            !text(&visible).contains('\x1b'),
            "Plain output cannot contain ANSI"
        );
    }
}
#[test]
fn graph_preserves_peaks_and_gaps() {
    let values = (0..120)
        .map(|index| Some(if index == 0 { 99.0 } else { 40.0 }))
        .collect::<Vec<_>>();
    assert!(
        graph(&values, 20, 4)
            .first()
            .is_some_and(|line| line.contains("100")),
        "Old peaks must remain visible"
    );
    assert!(
        graph(&[Some(50.0), None, Some(60.0)], 2, 3)
            .iter()
            .any(|line| line.contains('·')),
        "Missing samples must remain visible"
    );
}

#[test]
fn system_values_are_not_parsed_as_formatted_lines() {
    let info = SystemInfo {
        hostname: Some("host · another host".into()),
        os: Some("OS · edition".into()),
        cpu: Some("CPU brand · variant".into()),
        kernel: Some("Kernel release".into()),
        architecture: "aarch64".into(),
        ..SystemInfo::default()
    };
    let rendered = text(&rows(
        &view(&info, &Snapshot::default(), &History::new()),
        160,
        true,
        Output::Plain,
    ));
    for value in [
        "host · another host",
        "OS · edition",
        "CPU brand · variant",
        "Kernel release",
    ] {
        assert!(
            rendered.contains(value),
            "Structured value must remain intact: {value}"
        );
    }
    assert!(
        !rendered.contains('\x1b'),
        "Plain output must contain no escape sequences"
    );
}

#[test]
fn paired_rows_preserve_both_sides_and_empty_tails() {
    use crossterm::style::Color;
    let row = |text: &str| Row::new(text.into(), Color::White);
    assert_eq!(
        text(&pair(vec![row("界"), row("tail")], vec![row("right")], 6)),
        "界      right\ntail    "
    );
    assert_eq!(
        text(&pair(Vec::new(), vec![row("right"), row("next")], 6)),
        "        right\n        next"
    );
    assert!(
        pair(Vec::new(), Vec::new(), 6).is_empty(),
        "Empty panels have no rows"
    );
}
