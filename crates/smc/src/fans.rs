//! Reading fans and sensors: detecting the type of the RPM keys and which key
//! controls manual mode.

use crate::keys::{self, *};
use crate::protocol::{Smc, Transport};
use crate::value::Value;
use crate::{known_sensor_label, FourCC, SmcError};

/// How this Mac exposes manual fan mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeKey {
    /// Per-fan `F{n}Md` (T2 models).
    PerFan,
    /// `FS! ` bitmask (pre-T2 models).
    Bitmask,
    /// Neither key exists.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanMode {
    Auto,
    Manual,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanInfo {
    pub index: u8,
    pub name: Option<String>,
    pub actual_rpm: Option<f64>,
    pub min_rpm: Option<f64>,
    pub max_rpm: Option<f64>,
    pub target_rpm: Option<f64>,
    pub safe_rpm: Option<f64>,
    /// Type of the RPM keys (`fpe2` or `flt `).
    pub rpm_type: Option<FourCC>,
    pub mode: FanMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanReport {
    pub mode_key: ModeKey,
    /// Does `Ftst` exist on this Mac?
    pub has_ftst: bool,
    pub fans: Vec<FanInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sensor {
    pub key: FourCC,
    pub label: Option<&'static str>,
    pub data_type: FourCC,
    pub celsius: f64,
}

fn optional<T>(r: Result<T, SmcError>) -> Result<Option<T>, SmcError> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(SmcError::KeyNotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Fan name from `F{n}ID`. On Intel Macs the name usually starts at byte 4.
fn fan_name(value: &Value) -> Option<String> {
    let Value::Bytes(b) = value else { return None };
    let tail = b.get(4..)?;
    let name: String = tail
        .iter()
        .take_while(|&&c| c != 0)
        .filter(|c| c.is_ascii_graphic() || **c == b' ')
        .map(|&c| c as char)
        .collect();
    let name = name.trim().to_string();
    (!name.is_empty()).then_some(name)
}

impl<T: Transport> Smc<T> {
    pub fn fan_count(&mut self) -> Result<u8, SmcError> {
        Ok(self.read_f64(FAN_COUNT)? as u8)
    }

    /// Which mode key this Mac uses: `F0Md` first, then `FS! `.
    pub fn detect_mode_key(&mut self) -> Result<ModeKey, SmcError> {
        if self.has_key(FourCC::fan(0, SUFFIX_MODE)?)? {
            Ok(ModeKey::PerFan)
        } else if self.has_key(FAN_FORCE_BITMASK)? {
            Ok(ModeKey::Bitmask)
        } else {
            Ok(ModeKey::Unknown)
        }
    }

    pub fn fan_mode(&mut self, index: u8, mode_key: ModeKey) -> Result<FanMode, SmcError> {
        let manual = match mode_key {
            ModeKey::PerFan => self.read_f64(FourCC::fan(index, SUFFIX_MODE)?)? as u64 & 1 == 1,
            ModeKey::Bitmask => (self.read_f64(FAN_FORCE_BITMASK)? as u64 >> index) & 1 == 1,
            ModeKey::Unknown => return Ok(FanMode::Unknown),
        };
        Ok(if manual {
            FanMode::Manual
        } else {
            FanMode::Auto
        })
    }

    /// Turn a fan's manual mode on/off. Only writes `FS! ` or `F{n}Md`.
    /// Requires root.
    pub fn set_fan_manual(
        &mut self,
        index: u8,
        mode_key: ModeKey,
        manual: bool,
    ) -> Result<(), SmcError> {
        match mode_key {
            ModeKey::PerFan => {
                let key = FourCC::fan(index, SUFFIX_MODE)?;
                let info = self.key_info(key)?;
                let bytes = crate::value::encode_unsigned(&info, manual as u64)?;
                self.write_raw(key, &bytes)
            }
            ModeKey::Bitmask => {
                if index > 15 {
                    return Err(SmcError::InvalidValue(format!(
                        "fan {index} outside the bitmask"
                    )));
                }
                let current = self.read_f64(FAN_FORCE_BITMASK)? as u64;
                let bit = 1u64 << index;
                let next = if manual {
                    current | bit
                } else {
                    current & !bit
                };
                let info = self.key_info(FAN_FORCE_BITMASK)?;
                let bytes = crate::value::encode_unsigned(&info, next)?;
                self.write_raw(FAN_FORCE_BITMASK, &bytes)
            }
            ModeKey::Unknown => Err(SmcError::InvalidValue(
                "this Mac does not expose a manual-mode key".into(),
            )),
        }
    }

    /// Write the target speed (`F{n}Tg`), refusing values outside `F{n}Mn..=F{n}Mx`.
    /// Requires root. The fan must be in manual mode for the value to take effect.
    pub fn set_fan_target(&mut self, index: u8, rpm: f64) -> Result<(), SmcError> {
        let min = self.read_f64(FourCC::fan(index, SUFFIX_MIN)?)?;
        let max = self.read_f64(FourCC::fan(index, SUFFIX_MAX)?)?;
        if !(min..=max).contains(&rpm) {
            return Err(SmcError::InvalidValue(format!(
                "{rpm:.0} RPM outside fan {index}'s range ({min:.0} to {max:.0})"
            )));
        }
        let key = FourCC::fan(index, SUFFIX_TARGET)?;
        let info = self.key_info(key)?;
        let bytes = crate::value::encode_rpm(&info, rpm)?;
        self.write_raw(key, &bytes)
    }

    pub fn fan_info(&mut self, index: u8, mode_key: ModeKey) -> Result<FanInfo, SmcError> {
        let actual_key = FourCC::fan(index, SUFFIX_ACTUAL)?;
        let rpm_type = optional(self.key_info(actual_key))?.map(|i| i.data_type);
        let mut rpm = |suffix| -> Result<Option<f64>, SmcError> {
            optional(self.read_f64(FourCC::fan(index, suffix)?))
        };
        let actual_rpm = rpm(SUFFIX_ACTUAL)?;
        let min_rpm = rpm(SUFFIX_MIN)?;
        let max_rpm = rpm(SUFFIX_MAX)?;
        let target_rpm = rpm(SUFFIX_TARGET)?;
        let safe_rpm = rpm(SUFFIX_SAFE)?;
        let name = optional(self.read(FourCC::fan(index, SUFFIX_ID)?))?
            .as_ref()
            .and_then(fan_name);
        let mode = self.fan_mode(index, mode_key).unwrap_or(FanMode::Unknown);
        Ok(FanInfo {
            index,
            name,
            actual_rpm,
            min_rpm,
            max_rpm,
            target_rpm,
            safe_rpm,
            rpm_type,
            mode,
        })
    }

    pub fn fan_report(&mut self) -> Result<FanReport, SmcError> {
        let mode_key = self.detect_mode_key()?;
        let has_ftst = self.has_key(keys::FAN_TEST)?;
        let count = self.fan_count()?.min(10);
        let fans = (0..count)
            .map(|i| self.fan_info(i, mode_key))
            .collect::<Result<_, _>>()?;
        Ok(FanReport {
            mode_key,
            has_ftst,
            fans,
        })
    }

    /// All temperature sensors (`T***` keys with a plausible reading).
    pub fn temperatures(&mut self) -> Result<Vec<Sensor>, SmcError> {
        let mut out = Vec::new();
        for key in self.all_keys()? {
            if key.to_bytes()[0] != b'T' {
                continue;
            }
            let Ok((info, bytes)) = self.read_raw(key) else {
                continue;
            };
            let t = info.data_type.to_bytes();
            let is_temp_type = t.starts_with(b"sp") || &t == b"flt " || t.starts_with(b"fp");
            if !is_temp_type {
                continue;
            }
            let Some(c) = crate::value::decode(info.data_type, &bytes).as_f64() else {
                continue;
            };
            if c > 0.0 && c < 130.0 {
                out.push(Sensor {
                    key,
                    label: known_sensor_label(key),
                    data_type: info.data_type,
                    celsius: c,
                });
            }
        }
        Ok(out)
    }
}
