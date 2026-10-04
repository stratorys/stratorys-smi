#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod system;

use crate::cli::Options;
use crate::source::Source;

pub fn sources(options: &Options) -> Vec<Box<dyn Source>> {
    let mut sources: Vec<Box<dyn Source>> = vec![Box::new(system::System::new())];
    #[cfg(target_os = "linux")]
    sources.extend(linux::sources(options));
    #[cfg(target_os = "macos")]
    sources.extend(macos::sources(options));
    sources
}
