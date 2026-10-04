use std::fmt;

use thiserror::Error;

use super::TelemetryState;

#[derive(Clone, Copy, Debug, Error, PartialEq)]
pub enum MeasurementError {
    #[error("Invalid {quantity}: {value}.")]
    Value { quantity: &'static str, value: f64 },
    #[error("Invalid memory usage: used={used}, free={free:?}, total={total} bytes.")]
    Memory {
        used: u64,
        free: Option<u64>,
        total: u64,
    },
}

impl MeasurementError {
    pub(super) fn sensor(value: f64) -> Self {
        Self::Value {
            quantity: "sensor reading",
            value,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Measurement<T> {
    WarmingUp,
    Available(T),
    #[default]
    Unavailable,
    Stale,
}

impl<T> Measurement<T> {
    pub fn available(self) -> Option<T> {
        match self {
            Self::Available(value) => Some(value),
            Self::WarmingUp | Self::Unavailable | Self::Stale => None,
        }
    }
    pub fn is_available(&self) -> bool { matches!(self, Self::Available(_)) }
    pub fn or(
        self,
        other: Self,
    ) -> Self {
        match self {
            Self::Unavailable => other,
            Self::WarmingUp | Self::Available(_) | Self::Stale => self,
        }
    }
    pub fn expire(
        self,
        state: TelemetryState,
    ) -> Self {
        match self {
            Self::Available(_) => match state {
                TelemetryState::WarmingUp => Self::WarmingUp,
                TelemetryState::Stale => Self::Stale,
            },
            Self::WarmingUp | Self::Unavailable | Self::Stale => self,
        }
    }
    pub fn validated<R>(value: Option<R>) -> Self
    where
        T: TryFrom<R, Error = MeasurementError>,
    {
        value.and_then(|value| T::try_from(value).ok()).into()
    }
    pub fn numeric(self) -> Option<f64>
    where
        T: Into<f64>,
    {
        self.available().map(Into::into)
    }
}
impl<T> From<Option<T>> for Measurement<T> {
    fn from(value: Option<T>) -> Self {
        match value {
            Some(value) => Self::Available(value),
            None => Self::Unavailable,
        }
    }
}
impl<T: fmt::Display> fmt::Display for Measurement<T> {
    fn fmt(
        &self,
        formatter: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::Available(value) => value.fmt(formatter),
            Self::WarmingUp | Self::Unavailable | Self::Stale => formatter.pad("N/A"),
        }
    }
}

macro_rules! quantity {
    ($name:ident($inner:ty), $quantity:literal, | $value:ident | $valid:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct $name($inner);
        impl TryFrom<$inner> for $name {
            type Error = MeasurementError;
            fn try_from($value: $inner) -> Result<Self, Self::Error> {
                if $valid {
                    Ok(Self($value))
                } else {
                    Err(MeasurementError::Value {
                        quantity: $quantity,
                        value: f64::from($value),
                    })
                }
            }
        }
        impl From<$name> for f64 {
            fn from(value: $name) -> Self { f64::from(value.0) }
        }
        impl fmt::Display for $name {
            fn fmt(
                &self,
                formatter: &mut fmt::Formatter<'_>,
            ) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}
quantity!(TemperatureCelsius(f64), "temperature in Celsius", |value| {
    super::valid_temperature(value)
});
quantity!(Percentage(f64), "percentage", |value| {
    value.is_finite() && (0.0..=100.0).contains(&value)
});
quantity!(PowerWatts(f64), "power in watts", |value| value.is_finite()
    && value >= 0.0);
quantity!(FrequencyMhz(u32), "frequency in MHz", |value| value > 0);
quantity!(Threshold(f64), "temperature threshold", |value| value
    .is_finite()
    && value > 0.0
    && value <= 200.0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bytes(pub u64);
impl Bytes {
    pub fn gib(self) -> f64 { self.0 as f64 / 1_073_741_824.0 }
    pub fn gb(self) -> f64 { self.0 as f64 / 1e9 }
    pub fn percent_of(
        self,
        total: Self,
    ) -> f64 {
        if total.0 == 0 {
            0.0
        } else {
            100.0 * self.0.min(total.0) as f64 / total.0 as f64
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryUsage {
    used: u64,
    total: u64,
}
impl MemoryUsage {
    pub fn new(
        used: u64,
        total: u64,
    ) -> Result<Self, MeasurementError> {
        if used <= total {
            Ok(Self {
                used,
                total,
            })
        } else {
            Err(MeasurementError::Memory {
                used,
                free: None,
                total,
            })
        }
    }
    pub fn used(self) -> Bytes { Bytes(self.used) }
    pub fn total(self) -> Bytes { Bytes(self.total) }
    pub fn used_percent(self) -> f64 { self.used().percent_of(self.total()) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuMemory {
    usage: MemoryUsage,
    free: u64,
    reserved: u64,
}
impl TryFrom<(u64, u64, u64, u64)> for GpuMemory {
    type Error = MeasurementError;
    fn try_from((used, reserved, free, total): (u64, u64, u64, u64)) -> Result<Self, Self::Error> {
        let usage = MemoryUsage::new(used, total)?;
        if total > 0
            && free <= total
            && used
                .checked_add(reserved)
                .and_then(|sum| sum.checked_add(free))
                .is_some_and(|sum| sum <= total)
        {
            Ok(Self {
                usage,
                free,
                reserved,
            })
        } else {
            Err(MeasurementError::Memory {
                used,
                free: Some(free),
                total,
            })
        }
    }
}
impl GpuMemory {
    pub fn used(self) -> Bytes { self.usage.used() }
    pub fn total(self) -> Bytes { self.usage.total() }
    pub fn free(self) -> Bytes { Bytes(self.free) }
    pub fn reserved(self) -> Bytes { Bytes(self.reserved) }
    pub fn occupied(self) -> Bytes { Bytes(self.usage.total - self.free) }
    pub fn occupied_percent(self) -> f64 { self.occupied().percent_of(self.total()) }
}

#[cfg(test)]
mod tests {
    use super::{
        Bytes,
        FrequencyMhz,
        Measurement,
        MemoryUsage,
        Percentage,
        PowerWatts,
        Threshold,
    };
    use crate::model::{
        GpuMemory,
        MeasurementError,
        TelemetryState,
    };

    fn gpu_memory(
        used: u64,
        reserved: u64,
        free: u64,
        total: u64,
    ) -> Result<GpuMemory, MeasurementError> {
        GpuMemory::try_from((used, reserved, free, total))
    }

    #[test]
    fn gpu_reserve_is_separate_from_allocations() {
        let mib = 1024 * 1024;
        let memory =
            gpu_memory(3 * mib, 86 * mib, 8103 * mib, 8192 * mib).expect("Valid device counters");
        assert_eq!(memory.used(), Bytes(3 * mib));
        assert_eq!(memory.reserved(), Bytes(86 * mib));
        assert_eq!(memory.occupied(), Bytes(89 * mib));
        assert!(gpu_memory(3, 8, 0, 10).is_err(), "Reserve must fit total");
        assert!(
            gpu_memory(1, u64::MAX, 0, u64::MAX).is_err(),
            "Reserve addition must not overflow"
        );
        assert!(gpu_memory(0, 0, 0, 0).is_err(), "Zero total is unavailable");
        let empty = gpu_memory(0, 0, 10, 10).expect("Empty device");
        assert_eq!(empty.occupied(), Bytes(0));
        assert!(
            gpu_memory(5, 0, 6, 10).is_err(),
            "Free and used memory must fit total"
        );
        assert!(
            gpu_memory(u64::MAX, 0, 1, u64::MAX).is_err(),
            "Memory addition must not overflow"
        );
        assert!(
            gpu_memory(5, 0, 4, 10).is_ok(),
            "Driver reserved memory may occupy the remainder"
        );
    }
    #[test]
    fn invalid_values_never_enter_the_domain() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.1, 100.1] {
            assert!(
                Percentage::try_from(value).is_err(),
                "Invalid percentage must fail: {value}"
            );
            assert_eq!(
                Measurement::<Percentage>::validated(Some(value)),
                Measurement::Unavailable
            );
        }
        for value in [0.0, 100.0] {
            assert!(
                Percentage::try_from(value).is_ok(),
                "Valid endpoint must pass: {value}"
            );
        }
        for value in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(
                PowerWatts::try_from(value).is_err(),
                "Invalid power must fail: {value}"
            );
        }
        assert!(PowerWatts::try_from(0.0).is_ok(), "Idle power can be zero");
        assert!(
            FrequencyMhz::try_from(0).is_err(),
            "Zero frequency is an unavailable sentinel"
        );
        assert!(
            FrequencyMhz::try_from(u32::MAX).is_ok(),
            "A source frequency needs no invented upper bound"
        );
        for sentinel in [65261.85, 0.0, f64::NAN] {
            assert!(
                Threshold::try_from(sentinel).is_err(),
                "Threshold sentinel must fail: {sentinel}"
            );
        }
        assert!(Threshold::try_from(81.85).is_ok(), "Real limit must pass");
        assert!(
            MemoryUsage::new(11, 10).is_err(),
            "Used bytes must not exceed total"
        );
        assert!(MemoryUsage::new(0, 0).is_ok(), "A host may have no swap");
    }
    #[test]
    fn unavailable_states_do_not_supply_samples() {
        for measurement in [
            Measurement::<f64>::WarmingUp,
            Measurement::Unavailable,
            Measurement::Stale,
        ] {
            assert!(
                !measurement.is_available(),
                "{measurement:?} is not a sample"
            );
            assert_eq!(measurement.available(), None);
            assert_eq!(measurement.expire(TelemetryState::Stale), measurement);
        }
        assert_eq!(Measurement::Available(0.0).available(), Some(0.0));
        assert_eq!(
            Measurement::Unavailable.or(Measurement::Available(1.0)),
            Measurement::Available(1.0)
        );
        assert_eq!(
            Measurement::Stale.or(Measurement::Available(1.0)),
            Measurement::Stale,
            "A reported state is kept"
        );
        assert_eq!(
            Measurement::Available(5.0).expire(TelemetryState::WarmingUp),
            Measurement::WarmingUp
        );
        assert_eq!(Bytes(1).percent_of(Bytes(0)), 0.0);
        assert_eq!(Bytes(3).percent_of(Bytes(2)), 100.0);
    }
}
