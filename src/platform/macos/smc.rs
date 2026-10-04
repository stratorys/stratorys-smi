use std::time::{
    Duration,
    Instant,
};

use macmon::sources::{
    SMC,
    SensorVal,
};

use crate::model::{
    Cause,
    Diagnostic,
    Operation,
    Problem,
    Reading,
    SensorCategory,
    SourceId,
    Unit,
    valid_temperature,
};
use crate::source::REDISCOVERY;

#[derive(Clone, Default)]
pub struct SensorBatch {
    pub readings: Vec<Reading>,
    pub notices: Vec<Diagnostic>,
}

const KEY_SCAN: u32 = 4096;
const KEYS: usize = 640;
pub const SENSOR_REFRESH: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SmcValue {
    Float(f32),
    Unsigned(u64),
    Signed(i64),
}
impl SmcValue {
    pub fn decode(
        data_type: &str,
        data: &[u8],
    ) -> Option<Self> {
        fn bytes<const N: usize>(data: &[u8]) -> Option<[u8; N]> { data.try_into().ok() }
        let unsigned = |width: usize| match (data.len() == width).then_some(width)? {
            1 => bytes(data).map(|value| u64::from(u8::from_be_bytes(value))),
            2 => bytes(data).map(|value| u64::from(u16::from_be_bytes(value))),
            4 => bytes(data).map(|value| u64::from(u32::from_be_bytes(value))),
            8 => bytes(data).map(u64::from_be_bytes),
            _ => None,
        };
        let signed = |width: usize| match (data.len() == width).then_some(width)? {
            1 => bytes(data).map(|value| i64::from(i8::from_be_bytes(value))),
            2 => bytes(data).map(|value| i64::from(i16::from_be_bytes(value))),
            4 => bytes(data).map(|value| i64::from(i32::from_be_bytes(value))),
            8 => bytes(data).map(i64::from_be_bytes),
            _ => None,
        };
        let hex = |digit: u8| char::from(digit).to_digit(16);
        match data_type.as_bytes() {
            b"flt " => bytes(data).map(|value| Self::Float(f32::from_ne_bytes(value))),
            b"hex_" => unsigned(data.len()).map(Self::Unsigned),
            b"ui8 " => unsigned(1).map(Self::Unsigned),
            b"ui16" => unsigned(2).map(Self::Unsigned),
            b"ui32" => unsigned(4).map(Self::Unsigned),
            b"ui64" => unsigned(8).map(Self::Unsigned),
            b"si8 " => signed(1).map(Self::Signed),
            b"si16" => signed(2).map(Self::Signed),
            b"si32" => signed(4).map(Self::Signed),
            b"si64" => signed(8).map(Self::Signed),
            [b'f', b'p', integer, fraction] if hex(*integer)? + hex(*fraction)? == 16 => {
                let scale = f32::from(1u16 << hex(*fraction)?);
                bytes(data).map(|value| Self::Float(f32::from(u16::from_be_bytes(value)) / scale))
            }
            [b's', b'p', integer, fraction] if hex(*integer)? + hex(*fraction)? == 15 => {
                let scale = f32::from(1u16 << hex(*fraction)?);
                bytes(data).map(|value| Self::Float(f32::from(i16::from_be_bytes(value)) / scale))
            }
            _ => None,
        }
    }
    pub fn as_f64(self) -> f64 {
        match self {
            Self::Float(value) => f64::from(value),
            Self::Unsigned(value) => value as f64,
            Self::Signed(value) => value as f64,
        }
    }
}

fn unit(key: &str) -> Option<Unit> {
    if key.starts_with('T') {
        Some(Unit::Celsius)
    } else if key.starts_with('F') && key.ends_with("Ac") {
        Some(Unit::Rpm)
    } else if matches!(key, "PSTR" | "PCPC" | "PG0R") {
        Some(Unit::Watts)
    } else if matches!(key, "VC0C" | "VG0C") {
        Some(Unit::Volts)
    } else {
        None
    }
}
fn reading(value: &SensorVal) -> Option<Reading> {
    let key = value.name.as_str();
    let unit = unit(key)?;
    let number = SmcValue::decode(&value.unit, &value.data)?.as_f64();
    if !number.is_finite()
        || (unit == Unit::Celsius && (!valid_temperature(number) || number == 0.0))
    {
        return None;
    }
    let (family, category) = if ["Tp", "Te", "TC"]
        .iter()
        .any(|prefix| key.starts_with(prefix))
    {
        ("CPU", SensorCategory::Cpu)
    } else if key.starts_with("Tg") || key.starts_with("TG") {
        ("GPU", SensorCategory::Gpu)
    } else if unit == Unit::Rpm {
        ("Fan", SensorCategory::Fan)
    } else {
        (
            "SMC",
            match key {
                "PCPC" | "VC0C" => SensorCategory::Cpu,
                "PG0R" | "VG0C" => SensorCategory::Gpu,
                "PSTR" => SensorCategory::System,
                _ => SensorCategory::Other,
            },
        )
    };
    Reading::new(
        format!("{family} / {key}"),
        number,
        unit,
        category,
        SourceId::Smc,
    )
    .ok()
}

pub struct SmcSensors {
    keys: Vec<String>,
    discovery: Option<Diagnostic>,
    discovered: Option<Instant>,
    sampled: Option<Instant>,
    cached: SensorBatch,
}
impl SmcSensors {
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            discovery: None,
            discovered: None,
            sampled: None,
            cached: SensorBatch::default(),
        }
    }
    pub fn sample(
        &mut self,
        smc: Result<&mut SMC, &Diagnostic>,
    ) -> SensorBatch {
        let smc = match smc {
            Ok(smc) => smc,
            Err(notice) => {
                return SensorBatch {
                    readings: Vec::new(),
                    notices: vec![notice.clone()],
                };
            }
        };
        if self
            .discovered
            .is_none_or(|time| time.elapsed() >= REDISCOVERY)
        {
            (self.keys, self.discovery) = match discover(smc) {
                Ok(mut keys) => {
                    let limited = keys.len() > KEYS;
                    keys.truncate(KEYS);
                    (
                        keys,
                        limited.then(|| {
                            Diagnostic::new(
                                SourceId::Smc,
                                Problem::Limit {
                                    max: KEYS,
                                },
                            )
                        }),
                    )
                }
                Err(notice) => (Vec::new(), Some(notice)),
            };
            self.discovered = Some(Instant::now());
            self.sampled = None;
        }
        if self
            .sampled
            .is_none_or(|time| time.elapsed() >= SENSOR_REFRESH)
        {
            let mut failure = None;
            let mut failures = 0usize;
            let readings = self
                .keys
                .iter()
                .filter_map(|key| match smc.read_val(key) {
                    Ok(value) => reading(&value),
                    Err(error) => {
                        failures += 1;
                        failure.get_or_insert_with(|| Cause::of(error.as_ref()));
                        None
                    }
                })
                .collect::<Vec<_>>();
            let failed = match failure {
                Some(cause) if failures == self.keys.len() => Some(Diagnostic::unavailable(
                    SourceId::Smc,
                    Operation::Sample,
                    cause,
                )),
                Some(_) => Some(Diagnostic::new(SourceId::Smc, Problem::Partial)),
                None => None,
            };
            self.cached = SensorBatch {
                notices: self.discovery.iter().cloned().chain(failed).collect(),
                readings,
            };
            self.sampled = Some(Instant::now());
        }
        self.cached.clone()
    }
}
fn discover(smc: &mut SMC) -> Result<Vec<String>, Diagnostic> {
    let count = smc.key_count().map_err(|error| {
        Diagnostic::unavailable(
            SourceId::Smc,
            Operation::Discover,
            Cause::of(error.as_ref()),
        )
    })?;
    Ok((0..count.min(KEY_SCAN))
        .filter_map(|index| smc.key_by_index(index).ok())
        .filter(|key| unit(key).is_some())
        .take(KEYS + 1)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{
        SmcValue,
        unit,
    };
    use crate::model::Unit;

    #[test]
    fn only_read_sensor_keys() {
        assert_eq!(unit("Tp09"), Some(Unit::Celsius));
        assert_eq!(unit("F0Ac"), Some(Unit::Rpm));
        assert_eq!(unit("F0Tg"), None);
    }
    #[test]
    fn decodes_smc_data_types() {
        assert_eq!(
            SmcValue::decode("flt ", &42.5f32.to_ne_bytes()),
            Some(SmcValue::Float(42.5))
        );
        assert_eq!(
            SmcValue::decode("ui16", &[0x01, 0x02]),
            Some(SmcValue::Unsigned(258))
        );
        assert_eq!(
            SmcValue::decode("si8 ", &[0xff]),
            Some(SmcValue::Signed(-1))
        );
        assert_eq!(
            SmcValue::decode("sp78", &[0x28, 0x80]),
            Some(SmcValue::Float(40.5))
        );
        assert_eq!(
            SmcValue::decode("fpe2", &[0x00, 0x08]),
            Some(SmcValue::Float(2.0))
        );
        assert_eq!(SmcValue::decode("ui16", &[0x01]), None, "Size must match");
        assert_eq!(SmcValue::decode("flag", &[1]), None);
        assert_eq!(SmcValue::decode("ch8*", b"abc"), None);
    }
}
