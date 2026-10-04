mod hwmon;
mod nvidia;
mod procfs;

use std::io::{
    self,
    Read,
};
use std::path::Path;
use std::time::Duration;

use self::hwmon::{
    Chips,
    Sensors,
};
use crate::cli::Options;
use crate::model::{
    Snapshot,
    SourceId,
};
use crate::source::{
    Background,
    Source,
};

pub fn sources(options: &Options) -> Vec<Box<dyn Source>> {
    let mut sources: Vec<Box<dyn Source>> = vec![
        Box::new(Sensors::new(Chips::Hardware)),
        Box::new(procfs::Host::new()),
        Box::new(nvidia::Nvidia::new()),
    ];
    if !options.no_smart {
        sources.push(Box::new(Background::spawn(
            SourceId::DriveTemp,
            Duration::from_secs(30),
            Duration::from_secs(60),
            Snapshot::default(),
            || Sensors::new(Chips::Drives),
        )));
    }
    sources
}

fn read_bounded(path: impl AsRef<Path>) -> io::Result<String> {
    const LIMIT: u64 = 64 * 1024;
    let mut text = String::with_capacity(256);
    std::fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Telemetry file exceeds 64 KiB",
        ));
    }
    Ok(text.trim().to_owned())
}
