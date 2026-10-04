use std::io::{
    self,
    IsTerminal,
};
use std::time::{
    Duration,
    Instant,
};

use crossterm::event::{
    self,
    Event,
    KeyCode,
    KeyEventKind,
    KeyModifiers,
    MouseEventKind,
};
use crossterm::terminal;
use signal_hook::consts::{
    SIGHUP,
    SIGINT,
    SIGQUIT,
    SIGTERM,
};
use signal_hook::iterator::Signals;

use crate::cli::Options;
use crate::model::{
    History,
    Snapshot,
    Uptime,
};
use crate::platform;
use crate::source::Collector;
use crate::ui::{
    self,
    Output,
    Terminal,
    View,
};

#[derive(Clone, Copy)]
enum PageMove {
    Next,
    Previous,
    First,
    Last,
}
impl PageMove {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "every other key leaves the page unchanged"
    )]
    fn from_key(code: KeyCode) -> Option<Self> {
        match code {
            KeyCode::Down | KeyCode::Right | KeyCode::PageDown => Some(Self::Next),
            KeyCode::Up | KeyCode::Left | KeyCode::PageUp => Some(Self::Previous),
            KeyCode::Home => Some(Self::First),
            KeyCode::End => Some(Self::Last),
            _ => None,
        }
    }
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "only the wheel changes the page"
    )]
    fn from_mouse(kind: MouseEventKind) -> Option<Self> {
        match kind {
            MouseEventKind::ScrollDown => Some(Self::Next),
            MouseEventKind::ScrollUp => Some(Self::Previous),
            _ => None,
        }
    }
    fn apply(
        self,
        page: usize,
    ) -> usize {
        match self {
            Self::Next => page.saturating_add(1),
            Self::Previous => page.saturating_sub(1),
            Self::First => 0,
            Self::Last => usize::MAX,
        }
    }
}

fn restore_on_signal() -> io::Result<()> {
    let mut signals = Signals::new([SIGTERM, SIGHUP, SIGINT, SIGQUIT])?;
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                let _output = io::stdout().lock();
                ui::restore();
                std::process::exit(128 + signal);
            }
        })
        .map(drop)
}

const WARM_UP: Duration = Duration::from_millis(1500);

pub(crate) struct App {
    collector: Collector,
    history: History,
    options: Options,
    page: usize,
    started: Instant,
}
impl App {
    pub(crate) fn new(options: Options) -> Self {
        Self {
            collector: Collector::new(platform::sources(&options)),
            history: History::new(),
            options,
            page: 0,
            started: Instant::now(),
        }
    }
    fn snapshot(&mut self) -> Snapshot {
        let sample = self.collector.sample();
        self.history.update_snapshot(Instant::now(), &sample);
        sample
    }
    fn view<'view>(
        &'view self,
        sample: &'view Snapshot,
    ) -> View<'view> {
        View {
            configuration: self.collector.configuration(),
            sample,
            history: &self.history,
            uptime: Uptime {
                host: sample.uptime,
                session: self.started.elapsed(),
            },
        }
    }
    pub(crate) fn run(mut self) -> io::Result<()> {
        if self.options.once || !io::stdout().is_terminal() || !io::stdin().is_terminal() {
            let _ = self.snapshot();
            std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
            self.collector.warm_up(WARM_UP);
            let sample = self.snapshot();
            let rows = ui::rows(
                &self.view(&sample),
                120,
                self.options.details,
                Output::Plain,
            );
            println!("{}", ui::text(&rows));
            return Ok(());
        }
        self.collector.warm_up(WARM_UP);
        let terminal = Terminal::open()?;
        restore_on_signal()?;
        self.dashboard(terminal)
    }
    fn dashboard(
        mut self,
        mut terminal: Terminal,
    ) -> io::Result<()> {
        loop {
            let started = Instant::now();
            let sample = self.snapshot();
            self.paint(&mut terminal, &sample)?;
            while let Some(remaining) = self.options.interval.checked_sub(started.elapsed()) {
                if !event::poll(remaining)? {
                    continue;
                }
                let action = match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Release => continue,
                    Event::Key(key)
                        if key.code == KeyCode::Char('q')
                            || (key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL)) =>
                    {
                        return Ok(());
                    }
                    Event::Key(key) => PageMove::from_key(key.code),
                    Event::Mouse(mouse) => PageMove::from_mouse(mouse.kind),
                    Event::Resize(_, _) => None,
                    Event::FocusGained | Event::FocusLost => continue,
                };
                if let Some(action) = action {
                    self.page = action.apply(self.page);
                }
                self.paint(&mut terminal, &sample)?;
            }
        }
    }
    fn paint(
        &mut self,
        terminal: &mut Terminal,
        sample: &Snapshot,
    ) -> io::Result<()> {
        let (width, height) = terminal::size()?;
        let canvas_width = usize::from(width);
        let height = usize::from(height);
        let width = canvas_width.min(ui::WIDTH_MAX);
        let view = self.view(sample);
        let minimal = if self.options.details {
            Vec::new()
        } else {
            ui::minimal_rows(&view, width)
        };
        let required = if self.options.details {
            ui::HEIGHT_MIN
        } else {
            ui::height_min(&minimal)
        };
        if canvas_width < ui::WIDTH_MIN || height < required {
            let prompt = ui::resize_prompt(canvas_width, height, required);
            return terminal.render(prompt, canvas_width, 0, canvas_width, height);
        }
        if self.options.details {
            let frame = ui::frame(&view, width, true, Output::Interactive);
            let rows = frame.rows;
            let pages = rows.len().div_ceil(height.saturating_sub(1).max(1));
            let page = self.page.min(pages.saturating_sub(1));
            let logo_rows = if page == 0 { frame.logo_rows } else { 0 };
            self.page = page;
            terminal.render(
                ui::viewport(rows, height, page),
                width,
                logo_rows,
                canvas_width,
                height,
            )
        } else {
            let frame = ui::overview(&view, width, height, minimal);
            terminal.render(frame.rows, width, frame.logo_rows, canvas_width, height)
        }
    }
}
