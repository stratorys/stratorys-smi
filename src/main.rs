#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("stratorys-smi supports Linux and macOS.");
#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
compile_error!("stratorys-smi supports macOS on Apple silicon only.");

mod app;
mod cli;
mod model;
mod platform;
mod source;
mod ui;

use clap::Parser;

use crate::app::App;
use crate::cli::Options;

fn install_panic_hook() {
    let main = std::thread::current().id();
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == main {
            ui::restore();
            report(info);
        }
    }));
}

fn main() -> std::process::ExitCode {
    install_panic_hook();
    match App::new(Options::parse()).run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("stratorys-smi: Terminal or collector failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
