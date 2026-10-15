//! SMC keys and types: 4-character codes (FourCC).

use std::fmt;
use std::str::FromStr;

use crate::SmcError;

/// 4-character ASCII code, stored as a big-endian `u32`, exactly as the SMC
/// expects it (`'F','0','A','c'` -> `0x46304163`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FourCC(pub u32);

impl FourCC {
    /// Build from 4 ASCII bytes.
    pub const fn from_bytes(b: [u8; 4]) -> Self {
        FourCC(u32::from_be_bytes(b))
    }

    pub const fn to_bytes(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    /// Per-fan key in the `F{}Ac` format etc. `index` must be 0..=9.
    pub fn fan(index: u8, suffix: &[u8; 2]) -> Result<Self, SmcError> {
        if index > 9 {
            return Err(SmcError::InvalidKey(format!(
                "fan index {index} out of 0..=9"
            )));
        }
        Ok(FourCC::from_bytes([
            b'F',
            b'0' + index,
            suffix[0],
            suffix[1],
        ]))
    }

    /// Textual representation (non-printable bytes become `.`).
    pub fn as_string(self) -> String {
        self.to_bytes()
            .iter()
            .map(|&c| {
                if (0x20..0x7f).contains(&c) {
                    c as char
                } else {
                    '.'
                }
            })
            .collect()
    }
}

impl FromStr for FourCC {
    type Err = SmcError;

    /// Accepts 1 to 4 ASCII characters; pads with trailing spaces
    /// (`"FS!"` becomes `"FS! "`, `"flt"` becomes `"flt "`).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if bytes.is_empty() || bytes.len() > 4 || !s.is_ascii() {
            return Err(SmcError::InvalidKey(s.to_string()));
        }
        let mut b = [b' '; 4];
        b[..bytes.len()].copy_from_slice(bytes);
        Ok(FourCC::from_bytes(b))
    }
}

impl fmt::Display for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_string())
    }
}

impl fmt::Debug for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FourCC({:?})", self.as_string())
    }
}

/// Known SMC keys used for fans and mode.
pub mod keys {
    use super::FourCC;

    /// Total number of keys in the SMC (`ui32`).
    pub const KEY_COUNT: FourCC = FourCC::from_bytes(*b"#KEY");
    /// Number of fans (`ui8`).
    pub const FAN_COUNT: FourCC = FourCC::from_bytes(*b"FNum");
    /// Manual-mode bitmask on non-T2 Macs (bit n = fan n).
    pub const FAN_FORCE_BITMASK: FourCC = FourCC::from_bytes(*b"FS! ");
    /// "Test"/unlock key; its exact use is not confirmed.
    pub const FAN_TEST: FourCC = FourCC::from_bytes(*b"Ftst");

    /// Suffixes of the per-fan keys (`F{n}xx`).
    pub const SUFFIX_ACTUAL: &[u8; 2] = b"Ac";
    pub const SUFFIX_MIN: &[u8; 2] = b"Mn";
    pub const SUFFIX_MAX: &[u8; 2] = b"Mx";
    pub const SUFFIX_TARGET: &[u8; 2] = b"Tg";
    pub const SUFFIX_SAFE: &[u8; 2] = b"Sf";
    pub const SUFFIX_ID: &[u8; 2] = b"ID";
    /// Per-fan manual mode (T2 Macs).
    pub const SUFFIX_MODE: &[u8; 2] = b"Md";
}

/// Labels for the most common CPU sensors.
pub fn known_sensor_label(key: FourCC) -> Option<&'static str> {
    Some(match &key.to_bytes() {
        b"TC0P" => "CPU Proximity",
        b"TC0D" => "CPU Diode",
        b"TC0E" => "CPU Diode (virtual)",
        b"TC0F" => "CPU Diode (filtered)",
        b"TC0H" => "CPU Heatsink",
        b"TCMX" => "CPU Core (max)",
        b"TCSA" => "CPU System Agent",
        b"TPCD" => "PCH Diode",
        b"TCXC" => "CPU PECI",
        b"TCGC" => "GPU PECI (integrated)",
        b"TG0D" => "GPU Diode",
        b"TG0P" => "GPU Proximity",
        b"TA0P" => "Ambient",
        b"TB0T" => "Battery",
        b"Th1H" => "Heatsink Proximity",
        b"Ts0P" => "Palm rest",
        b"TN0D" => "Northbridge Diode",
        b"TM0P" => "Memory Proximity",
        [b'T', b'C', d, b'C' | b'c'] if d.is_ascii_digit() => "CPU Core",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_roundtrip() {
        let k: FourCC = "F0Ac".parse().unwrap();
        assert_eq!(k.0, 0x4630_4163);
        assert_eq!(k.to_string(), "F0Ac");
    }

    #[test]
    fn fourcc_pads_with_spaces() {
        let k: FourCC = "FS!".parse().unwrap();
        assert_eq!(k, keys::FAN_FORCE_BITMASK);
        assert_eq!(k.to_string(), "FS! ");
    }

    #[test]
    fn fourcc_rejects_bad_input() {
        assert!("".parse::<FourCC>().is_err());
        assert!("ABCDE".parse::<FourCC>().is_err());
        assert!("Fã".parse::<FourCC>().is_err());
    }

    #[test]
    fn fan_key() {
        assert_eq!(
            FourCC::fan(1, keys::SUFFIX_TARGET).unwrap().to_string(),
            "F1Tg"
        );
        assert!(FourCC::fan(10, keys::SUFFIX_TARGET).is_err());
    }

    #[test]
    fn sensor_labels() {
        assert_eq!(
            known_sensor_label("TC0P".parse().unwrap()),
            Some("CPU Proximity")
        );
        assert_eq!(
            known_sensor_label("TC3c".parse().unwrap()),
            Some("CPU Core")
        );
        assert_eq!(known_sensor_label("ZZZZ".parse().unwrap()), None);
    }
}
