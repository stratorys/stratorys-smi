use std::time::Duration;

use clap::Parser;

#[derive(Parser)]
#[command(
    version,
    about = "Native hardware dashboard for Linux/NVIDIA and macOS (Apple silicon)",
    after_help = "q or Ctrl+C exits. Diagnostic paging requires --details."
)]
pub(crate) struct Options {
    #[arg(long, default_value = "1", value_name = "SECONDS", value_parser = parse_interval, help = "Refresh every 0.2–60 seconds")]
    pub(crate) interval: Duration,
    #[arg(long, help = "Print one plain-text snapshot")]
    pub(crate) once: bool,
    #[arg(long, help = "Skip Linux SATA temperature readings")]
    pub(crate) no_smart: bool,
    #[arg(long, help = "Show full sensor and diagnostic pages")]
    pub(crate) details: bool,
}
#[derive(Debug, thiserror::Error)]
enum IntervalError {
    #[error("interval must be a number")]
    Number(#[source] std::num::ParseFloatError),
    #[error("interval must be 0.2–60 seconds")]
    Range { seconds: f64 },
}
fn parse_interval(value: &str) -> Result<Duration, IntervalError> {
    let seconds = value.parse::<f64>().map_err(IntervalError::Number)?;
    if !seconds.is_finite() || !(0.2..=60.0).contains(&seconds) {
        return Err(IntervalError::Range {
            seconds,
        });
    }
    Ok(Duration::from_secs_f64(seconds))
}
#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Options;

    fn parse(arguments: &[&str]) -> Result<Options, clap::Error> {
        Options::try_parse_from(std::iter::once("stratorys-smi").chain(arguments.iter().copied()))
    }
    #[test]
    fn details_requires_explicit_flag() {
        assert!(
            parse(&[]).is_ok_and(|options| !options.details),
            "The default dashboard must not enable diagnostics"
        );
        assert!(
            parse(&["--details", "--once"]).is_ok_and(|options| options.details && options.once),
            "Details must be available only through their explicit flag"
        );
    }
    #[test]
    fn invalid_intervals() {
        for value in ["NaN", "inf", "0", "-1", "100"] {
            assert!(
                parse(&["--interval", value]).is_err(),
                "Invalid interval must fail: {value}"
            );
        }
        assert!(
            parse(&["--interval"]).is_err(),
            "Missing interval must fail"
        );
    }
    #[test]
    fn unknown_argument() {
        assert!(parse(&["--something"]).is_err(), "Unknown option must fail");
    }
}
