mod charts;
mod layout;
mod logo;
mod panels;
mod primitives;
mod terminal;

use crossterm::style::Color;

pub(crate) use self::layout::{
    HEIGHT_MIN,
    Output,
    WIDTH_MAX,
    WIDTH_MIN,
    frame,
    height_min,
    overview,
    resize_prompt,
    rows,
    viewport,
};
pub(crate) use self::panels::minimal_rows;
pub(crate) use self::primitives::text;
pub(crate) use self::terminal::{
    Terminal,
    restore,
};
use crate::model::{
    History,
    Snapshot,
    SystemInfo,
    Uptime,
};

pub(crate) struct View<'view> {
    pub configuration: &'view SystemInfo,
    pub sample: &'view Snapshot,
    pub history: &'view History,
    pub uptime: Uptime,
}

const ACCENT: Color = Color::Rgb {
    r: 96,
    g: 165,
    b: 250,
};
const MUTED: Color = Color::Rgb {
    r: 148,
    g: 163,
    b: 184,
};

#[cfg(test)]
mod tests;
