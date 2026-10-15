//! Decoding and encoding of the SMC data formats.
//!
//! Relevant formats on Intel Macs:
//! - `fpe2`: unsigned fixed-point RPM, 2 bytes big-endian, 2 fraction bits (older Macs).
//! - `flt `: 4-byte float, native x86 order (little-endian) (T2 Macs).
//! - `sp78`: signed fixed-point temperature, 2 bytes big-endian, 8 fraction bits.
//! - `fpXY` / `spXY`: generic family; Y (hex) = fraction bits.
//! - `ui8 `, `ui16`, `ui32`, `si8 `, `si16`, `flag`: big-endian integers.

use crate::{FourCC, SmcError};

/// Type information for a key (SMC command 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInfo {
    pub size: u32,
    pub data_type: FourCC,
    pub attributes: u8,
}

/// Decoded value of a key.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    Flag(bool),
    /// Unknown type or structure (`{fds`, `ch8*`, …): raw bytes.
    Bytes(Vec<u8>),
}

impl Value {
    /// Value as a number, when it makes sense.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::Unsigned(v) => Some(v as f64),
            Value::Signed(v) => Some(v as f64),
            Value::Float(v) => Some(v),
            Value::Flag(b) => Some(if b { 1.0 } else { 0.0 }),
            Value::Bytes(_) => None,
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Unsigned(v) => write!(f, "{v}"),
            Value::Signed(v) => write!(f, "{v}"),
            Value::Float(v) => write!(f, "{v:.2}"),
            Value::Flag(b) => write!(f, "{b}"),
            Value::Bytes(b) => {
                let printable: String = b
                    .iter()
                    .take_while(|&&c| c != 0)
                    .map(|&c| {
                        if (0x20..0x7f).contains(&c) {
                            c as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                write!(f, "0x")?;
                for byte in b {
                    write!(f, "{byte:02x}")?;
                }
                if printable.len() >= 3 {
                    write!(f, " ({printable:?})")?;
                }
                Ok(())
            }
        }
    }
}

fn hex_digit(c: u8) -> Option<u32> {
    (c as char).to_digit(16)
}

fn be_uint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64)
}

/// Decode `bytes` according to the type reported by the SMC.
pub fn decode(data_type: FourCC, bytes: &[u8]) -> Value {
    let t = data_type.to_bytes();
    let raw = || Value::Bytes(bytes.to_vec());
    match &t {
        b"flt " if bytes.len() == 4 => {
            Value::Float(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64)
        }
        b"flag" if !bytes.is_empty() => Value::Flag(bytes[0] != 0),
        b"ui8 " | b"ui16" | b"ui32" | b"ui64" if !bytes.is_empty() && bytes.len() <= 8 => {
            Value::Unsigned(be_uint(bytes))
        }
        b"si8 " if bytes.len() == 1 => Value::Signed(bytes[0] as i8 as i64),
        b"si16" if bytes.len() == 2 => {
            Value::Signed(i16::from_be_bytes([bytes[0], bytes[1]]) as i64)
        }
        b"si32" if bytes.len() == 4 => {
            Value::Signed(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64)
        }
        [b'f', b'p', i, f] if bytes.len() == 2 => match (hex_digit(*i), hex_digit(*f)) {
            (Some(_), Some(frac)) => Value::Float(
                u16::from_be_bytes([bytes[0], bytes[1]]) as f64 / (1u32 << frac) as f64,
            ),
            _ => raw(),
        },
        [b's', b'p', i, f] if bytes.len() == 2 => match (hex_digit(*i), hex_digit(*f)) {
            (Some(_), Some(frac)) => Value::Float(
                i16::from_be_bytes([bytes[0], bytes[1]]) as f64 / (1u32 << frac) as f64,
            ),
            _ => raw(),
        },
        _ => raw(),
    }
}

/// Encode a fan speed (RPM) in the key's format.
pub fn encode_rpm(info: &KeyInfo, rpm: f64) -> Result<Vec<u8>, SmcError> {
    if !rpm.is_finite() || rpm < 0.0 {
        return Err(SmcError::InvalidValue(format!("invalid RPM: {rpm}")));
    }
    let t = info.data_type.to_bytes();
    match (&t, info.size) {
        (b"flt ", 4) => Ok((rpm as f32).to_le_bytes().to_vec()),
        ([b'f', b'p', _, f], 2) => {
            let frac = hex_digit(*f).ok_or(SmcError::UnsupportedType(info.data_type))?;
            let raw = (rpm * (1u32 << frac) as f64).round();
            if raw > u16::MAX as f64 {
                return Err(SmcError::InvalidValue(format!(
                    "RPM {rpm} does not fit in {}",
                    info.data_type
                )));
            }
            Ok((raw as u16).to_be_bytes().to_vec())
        }
        _ => Err(SmcError::UnsupportedType(info.data_type)),
    }
}

/// Encode an unsigned integer in the key's size (big-endian).
pub fn encode_unsigned(info: &KeyInfo, value: u64) -> Result<Vec<u8>, SmcError> {
    let size = info.size as usize;
    if size == 0 || size > 8 || (size < 8 && value >= 1u64 << (size * 8)) {
        return Err(SmcError::InvalidValue(format!(
            "{value} does not fit in {size} byte(s) ({})",
            info.data_type
        )));
    }
    Ok(value.to_be_bytes()[8 - size..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> FourCC {
        s.parse().unwrap()
    }

    #[test]
    fn fpe2_matches_smc_tool_example() {
        // 3500 RPM = 0x36b0 in fpe2.
        assert_eq!(decode(t("fpe2"), &[0x36, 0xb0]), Value::Float(3500.0));
        let info = KeyInfo {
            size: 2,
            data_type: t("fpe2"),
            attributes: 0,
        };
        assert_eq!(encode_rpm(&info, 3500.0).unwrap(), vec![0x36, 0xb0]);
    }

    #[test]
    fn flt_is_little_endian() {
        let bytes = 2000.0f32.to_le_bytes();
        assert_eq!(decode(t("flt"), &bytes), Value::Float(2000.0));
        let info = KeyInfo {
            size: 4,
            data_type: t("flt"),
            attributes: 0,
        };
        assert_eq!(encode_rpm(&info, 2000.0).unwrap(), bytes.to_vec());
    }

    #[test]
    fn sp78_temperature() {
        // 0x3200 = 50.0 °C; 0x3280 = 50.5 °C; negative works too.
        assert_eq!(decode(t("sp78"), &[0x32, 0x00]), Value::Float(50.0));
        assert_eq!(decode(t("sp78"), &[0x32, 0x80]), Value::Float(50.5));
        assert_eq!(decode(t("sp78"), &[0xff, 0x00]), Value::Float(-1.0));
    }

    #[test]
    fn integers_and_flags() {
        assert_eq!(decode(t("ui8"), &[2]), Value::Unsigned(2));
        assert_eq!(decode(t("ui16"), &[0x00, 0x03]), Value::Unsigned(3));
        assert_eq!(
            decode(t("ui32"), &[0, 0, 0x04, 0xd2]),
            Value::Unsigned(1234)
        );
        assert_eq!(decode(t("si8"), &[0xff]), Value::Signed(-1));
        assert_eq!(decode(t("flag"), &[1]), Value::Flag(true));
    }

    #[test]
    fn unknown_type_keeps_bytes() {
        assert_eq!(decode(t("{fds"), &[1, 2]), Value::Bytes(vec![1, 2]));
        // a size that does not match the type also falls back to raw bytes
        assert_eq!(decode(t("fpe2"), &[1, 2, 3]), Value::Bytes(vec![1, 2, 3]));
    }

    #[test]
    fn encode_rejects_bad_values() {
        let fpe2 = KeyInfo {
            size: 2,
            data_type: t("fpe2"),
            attributes: 0,
        };
        assert!(encode_rpm(&fpe2, -1.0).is_err());
        assert!(encode_rpm(&fpe2, 20000.0).is_err()); // 20000*4 > u16::MAX
        let sp78 = KeyInfo {
            size: 2,
            data_type: t("sp78"),
            attributes: 0,
        };
        assert!(encode_rpm(&sp78, 1000.0).is_err());
    }

    #[test]
    fn encode_unsigned_sizes() {
        let ui16 = KeyInfo {
            size: 2,
            data_type: t("ui16"),
            attributes: 0,
        };
        assert_eq!(encode_unsigned(&ui16, 3).unwrap(), vec![0, 3]);
        let ui8 = KeyInfo {
            size: 1,
            data_type: t("ui8"),
            attributes: 0,
        };
        assert!(encode_unsigned(&ui8, 256).is_err());
    }
}
