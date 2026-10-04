use std::io::{
    self,
    Stdout,
    Write,
};
use std::sync::{
    Mutex,
    PoisonError,
};

use crossterm::cursor::{
    Hide,
    MoveTo,
    Show,
};
use crossterm::event::{
    DisableMouseCapture,
    EnableMouseCapture,
};
use crossterm::style::{
    Attribute,
    Print,
    ResetColor,
    SetAttribute,
    SetForegroundColor,
};
use crossterm::terminal::{
    self,
    Clear,
    ClearType,
    EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{
    execute,
    queue,
};
use unicode_width::UnicodeWidthStr;

use super::layout::LOGO_COLUMNS;
use super::logo::{
    ImageProtocol,
    LogoLayout,
    image_protocol,
    logo_layout,
};
use super::primitives::{
    Row,
    clip,
};

const KITTY_DELETE: &str = "\x1b_Ga=d,d=I,i=7353,q=2\x1b\\";

static OPEN: Mutex<Option<ImageProtocol>> = Mutex::new(None);

pub fn restore() {
    let Some(protocol) = OPEN.lock().unwrap_or_else(PoisonError::into_inner).take() else {
        return;
    };
    let mut output = io::stdout().lock();
    if protocol == ImageProtocol::Kitty {
        let _ = write!(output, "{KITTY_DELETE}");
    }
    let _ = execute!(
        output,
        ResetColor,
        Show,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
}

fn cell(position: usize) -> io::Result<u16> {
    u16::try_from(position).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

pub struct Terminal {
    output: Stdout,
    previous: Vec<Row>,
    width: usize,
    logo: bool,
    image_protocol: ImageProtocol,
    canvas_width: usize,
    canvas_height: usize,
    logo_layout: LogoLayout,
}
impl Terminal {
    pub fn open() -> io::Result<Self> {
        Self {
            output: io::stdout(),
            previous: Vec::new(),
            width: 0,
            canvas_width: 0,
            canvas_height: 0,
            logo_layout: logo_layout(None),
            logo: false,
            image_protocol: image_protocol(
                &std::env::var("TERM_PROGRAM").unwrap_or_default(),
                &std::env::var("TERM").unwrap_or_default(),
                std::env::var_os("TMUX").is_some(),
            ),
        }
        .enter()
    }
    fn enter(mut self) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        *OPEN.lock().unwrap_or_else(PoisonError::into_inner) = Some(self.image_protocol);
        execute!(
            self.output,
            EnterAlternateScreen,
            EnableMouseCapture,
            Hide,
            Clear(ClearType::All)
        )?;
        Ok(self)
    }
    pub fn render(
        &mut self,
        rows: Vec<Row>,
        width: usize,
        logo_rows: usize,
        canvas_width: usize,
        canvas_height: usize,
    ) -> io::Result<()> {
        let mut output = self.output.lock();
        let logo = self.image_protocol != ImageProtocol::None && logo_rows > 0;
        let margin = canvas_width.saturating_sub(width) / 2;
        let margin_top = canvas_height.saturating_sub(rows.len()) / 2;
        let placement = logo_layout(terminal::window_size().ok().as_ref());
        let clear = self.width != width
            || self.canvas_width != canvas_width
            || self.canvas_height != canvas_height
            || self.previous.len() != rows.len()
            || self.logo_layout != placement
            || self.logo != logo;
        if clear {
            if self.image_protocol == ImageProtocol::Kitty {
                write!(output, "{KITTY_DELETE}")?;
            }
            queue!(output, MoveTo(0, 0), Clear(ClearType::All))?;
        }
        rows.iter().enumerate().try_for_each(|(index, row)| {
            if !clear && self.previous.get(index) == Some(row) {
                return Ok(());
            }
            let beside_logo = index < logo_rows;
            let start = if logo && beside_logo { LOGO_COLUMNS } else { 0 };
            queue!(
                output,
                MoveTo(cell(start + margin)?, cell(index + margin_top)?),
                Clear(ClearType::UntilNewLine)
            )?;
            if beside_logo {
                queue!(output, SetAttribute(Attribute::Bold))?;
            }
            row.spans
                .iter()
                .skip(usize::from(start > 0))
                .try_fold(width.saturating_sub(start), |remaining, span| {
                    let text = clip(&span.text, remaining);
                    queue!(output, SetForegroundColor(span.color), Print(&text))?;
                    Ok::<_, io::Error>(remaining.saturating_sub(text.width()))
                })
                .map(|_| ())?;
            queue!(output, SetAttribute(Attribute::NormalIntensity))?;
            Ok::<_, io::Error>(())
        })?;
        if logo && clear {
            let LogoLayout {
                columns,
                left,
                top,
                pixel_offset,
            } = placement;
            queue!(
                output,
                MoveTo(cell(margin + left)?, cell(margin_top + top)?)
            )?;
            match self.image_protocol {
                ImageProtocol::Kitty => {
                    let bytes = include_str!("../../assets/stratorys.png.base64").as_bytes();
                    let count = bytes.len().div_ceil(4096);
                    bytes
                        .chunks(4096)
                        .enumerate()
                        .try_for_each(|(index, chunk)| {
                            let payload = std::str::from_utf8(chunk).map_err(|error| {
                                io::Error::new(io::ErrorKind::InvalidData, error)
                            })?;
                            let continuation = usize::from(index + 1 < count);
                            if index == 0 {
                                write!(
                                    output,
                                    "\x1b_Ga=T,f=100,i=7353,c={columns},Y={pixel_offset},C=1,q=2,\
                                     m={continuation};{payload}\x1b\\"
                                )
                            } else {
                                write!(output, "\x1b_Gm={continuation};{payload}\x1b\\")
                            }
                        })?;
                }
                ImageProtocol::Iterm => write!(
                    output,
                    "\x1b]1337;File=inline=1;width={columns};height=auto;preserveAspectRatio=1;\
                     doNotMoveCursor=1:{}\x07",
                    include_str!("../../assets/stratorys.png.base64")
                )?,
                ImageProtocol::None => {}
            }
        }
        output.flush()?;
        self.previous = rows;
        self.width = width;
        self.canvas_width = canvas_width;
        self.canvas_height = canvas_height;
        self.logo_layout = placement;
        self.logo = logo;
        Ok(())
    }
}
impl Drop for Terminal {
    fn drop(&mut self) { restore(); }
}
