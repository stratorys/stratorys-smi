use crossterm::terminal;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ImageProtocol {
    None,
    Kitty,
    Iterm,
}
pub(super) fn image_protocol(
    program: &str,
    terminal: &str,
    multiplexer: bool,
) -> ImageProtocol {
    if multiplexer {
        ImageProtocol::None
    } else if program == "ghostty" || terminal == "xterm-ghostty" || terminal == "xterm-kitty" {
        ImageProtocol::Kitty
    } else if program == "iTerm.app" {
        ImageProtocol::Iterm
    } else {
        ImageProtocol::None
    }
}
#[derive(Clone, Copy, PartialEq)]
pub(super) struct LogoLayout {
    pub(super) columns: usize,
    pub(super) left: usize,
    pub(super) top: usize,
    pub(super) pixel_offset: usize,
}
struct CellSize {
    width: f64,
    height: f64,
}
pub(super) fn logo_layout(size: Option<&terminal::WindowSize>) -> LogoLayout {
    let dimensions =
        size.filter(|size| size.width > 0 && size.height > 0 && size.columns > 0 && size.rows > 0);
    let cell = dimensions
        .map(|size| CellSize {
            width: f64::from(size.width) / f64::from(size.columns),
            height: f64::from(size.height) / f64::from(size.rows),
        })
        .unwrap_or(CellSize {
            width: 1.0,
            height: 2.0,
        });
    let columns = (12.0 * cell.height / cell.width).floor().clamp(1.0, 24.0) as usize;
    let padding = (12.0 * cell.height - columns as f64 * cell.width).max(0.0) / 2.0;
    let top = (padding / cell.height).floor() as usize;
    LogoLayout {
        columns,
        left: (24 - columns) / 2,
        top,
        pixel_offset: (padding - top as f64 * cell.height).floor() as usize,
    }
}
