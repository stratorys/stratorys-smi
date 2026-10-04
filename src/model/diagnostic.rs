use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceId {
    #[cfg(target_os = "linux")]
    Hwmon,
    #[cfg(target_os = "linux")]
    Nvml,
    #[cfg(target_os = "linux")]
    DriveTemp,
    #[cfg(target_os = "macos")]
    Smc,
    #[cfg(target_os = "macos")]
    IoKit,
    #[cfg(target_os = "macos")]
    IoReport,
}
impl fmt::Display for SourceId {
    fn fmt(
        &self,
        formatter: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        formatter.write_str(match self {
            #[cfg(target_os = "linux")]
            Self::Hwmon => "Linux sensors",
            #[cfg(target_os = "linux")]
            Self::Nvml => "NVIDIA",
            #[cfg(target_os = "linux")]
            Self::DriveTemp => "SATA drivetemp",
            #[cfg(target_os = "macos")]
            Self::Smc => "SMC",
            #[cfg(target_os = "macos")]
            Self::IoKit => "IOKit",
            #[cfg(target_os = "macos")]
            Self::IoReport => "Apple runtime telemetry",
        })
    }
}

#[cfg(test)]
impl SourceId {
    #[cfg(target_os = "linux")]
    pub const SENSOR: Self = Self::Hwmon;
    #[cfg(target_os = "macos")]
    pub const SENSOR: Self = Self::IoKit;
    #[cfg(target_os = "linux")]
    pub const OTHER: Self = Self::Nvml;
    #[cfg(target_os = "macos")]
    pub const OTHER: Self = Self::Smc;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TelemetryState {
    WarmingUp,
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    Initialize,
    Discover,
    Sample,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cause(Vec<String>);
impl Cause {
    pub fn of(error: &(dyn std::error::Error + 'static)) -> Self {
        Self(
            std::iter::successors(Some(error), |error| error.source())
                .take(8)
                .map(|error| error.to_string().chars().take(512).collect())
                .collect(),
        )
    }
    pub fn context(
        self,
        context: impl fmt::Display,
    ) -> Self {
        Self(
            std::iter::once(context.to_string().chars().take(512).collect())
                .chain(self.0)
                .take(8)
                .collect(),
        )
    }
}
impl fmt::Display for Cause {
    fn fmt(
        &self,
        formatter: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        formatter.write_str(&self.0.join(": "))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    Unavailable { operation: Operation, cause: Cause },
    Panicked,
    Limit { max: usize },
    Partial,
    NoReadings,
    State(TelemetryState),
}
impl fmt::Display for Problem {
    fn fmt(
        &self,
        formatter: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::Unavailable {
                operation,
                cause,
            } => {
                let operation = match operation {
                    Operation::Initialize => "unavailable",
                    Operation::Discover => "discovery failed",
                    Operation::Sample => "sampling failed",
                };
                write!(formatter, "{operation}: {cause}")
            }
            Self::Panicked => {
                formatter.write_str("stopped after an unsupported hardware response.")
            }
            Self::Limit {
                max,
            } => write!(
                formatter,
                "some entries are unreadable or beyond the {max}-entry limit; discovery retries \
                 every 30 seconds."
            ),
            Self::Partial => formatter
                .write_str("some sensors are unavailable; discovery retries every 30 seconds."),
            Self::NoReadings => formatter.write_str("no sensors are exposed."),
            Self::State(TelemetryState::WarmingUp) => formatter.write_str("warming up."),
            Self::State(TelemetryState::Stale) => formatter.write_str("stale."),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub source: SourceId,
    pub problem: Problem,
}
impl Diagnostic {
    pub fn new(
        source: SourceId,
        problem: Problem,
    ) -> Self {
        Self {
            source,
            problem,
        }
    }
    pub fn unavailable(
        source: SourceId,
        operation: Operation,
        cause: Cause,
    ) -> Self {
        Self::new(
            source,
            Problem::Unavailable {
                operation,
                cause,
            },
        )
    }
    fn key(&self) -> (u8, SourceId, Option<Operation>) {
        let (rank, operation) = match &self.problem {
            Problem::Unavailable {
                operation, ..
            } => (0, Some(*operation)),
            Problem::Panicked => (1, None),
            Problem::Limit {
                ..
            } => (2, None),
            Problem::Partial => (3, None),
            Problem::NoReadings => (4, None),
            Problem::State(TelemetryState::Stale) => (5, None),
            Problem::State(TelemetryState::WarmingUp) => (6, None),
        };
        (rank, self.source, operation)
    }
}
impl fmt::Display for Diagnostic {
    fn fmt(
        &self,
        formatter: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        write!(formatter, "{}: {}", self.source, self.problem)
    }
}

pub fn deduplicate(notices: impl IntoIterator<Item = Diagnostic>) -> Vec<Diagnostic> {
    notices
        .into_iter()
        .fold(std::collections::BTreeMap::new(), |mut unique, notice| {
            unique.entry(notice.key()).or_insert(notice);
            unique
        })
        .into_values()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        Cause,
        Diagnostic,
        Operation,
        Problem,
        SourceId,
        TelemetryState,
        deduplicate,
    };

    #[derive(Debug, thiserror::Error)]
    #[error("{message}")]
    struct Chain {
        message: String,
        #[source]
        cause: Option<Box<Chain>>,
    }
    #[test]
    fn causes_are_bounded() {
        let error = (0..12).fold(
            Chain {
                message: "root".into(),
                cause: None,
            },
            |cause, index| Chain {
                message: format!("{index}:{}", "界".repeat(1000)),
                cause: Some(Box::new(cause)),
            },
        );
        let Cause(causes) = Cause::of(&error).context("GPU 0");
        assert_eq!(causes.len(), 8);
        assert_eq!(causes.first().map(String::as_str), Some("GPU 0"));
        assert!(
            causes.iter().all(|cause| cause.chars().count() <= 512),
            "Each cause must be bounded"
        );
    }
    #[test]
    fn deduplication_keeps_operations_apart_and_orders_by_severity() {
        let cause = || Cause::of(&std::io::Error::other("failure"));
        let notices = deduplicate([
            Diagnostic::new(SourceId::SENSOR, Problem::State(TelemetryState::WarmingUp)),
            Diagnostic::unavailable(SourceId::SENSOR, Operation::Sample, cause()),
            Diagnostic::unavailable(SourceId::SENSOR, Operation::Sample, cause()),
            Diagnostic::unavailable(SourceId::SENSOR, Operation::Initialize, cause()),
        ]);
        assert_eq!(notices.len(), 3);
        assert_eq!(
            notices.first().map(ToString::to_string).as_deref(),
            Some(format!("{}: unavailable: failure", SourceId::SENSOR).as_str())
        );
        assert_eq!(
            notices.last().map(|notice| &notice.problem),
            Some(&Problem::State(TelemetryState::WarmingUp))
        );
    }
}
