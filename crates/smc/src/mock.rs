//! Fake SMC to test the protocol and fan logic without hardware.
//! Available to other crates with the `testing` feature.

use crate::protocol::*;
use crate::{FourCC, SmcError};

pub struct MockSmc {
    /// (key, type, bytes) in index order.
    pub keys: Vec<(FourCC, FourCC, Vec<u8>)>,
    pub calls: Vec<u8>,
    /// Accepted writes, in order.
    pub writes: Vec<(FourCC, Vec<u8>)>,
}

fn k(s: &str) -> FourCC {
    s.parse().unwrap()
}

impl MockSmc {
    pub fn new(entries: &[(&str, &str, Vec<u8>)]) -> Self {
        let mut keys: Vec<_> = entries
            .iter()
            .map(|(n, t, b)| (k(n), k(t), b.clone()))
            .collect();
        let count = (keys.len() as u32 + 1).to_be_bytes().to_vec();
        keys.insert(0, (k("#KEY"), k("ui32"), count));
        MockSmc {
            keys,
            calls: Vec::new(),
            writes: Vec::new(),
        }
    }

    /// Change the value of an existing key (simulates the hardware).
    pub fn set(&mut self, key: &str, bytes: Vec<u8>) {
        let i = self.find(k(key).0).expect("key not present in mock");
        self.keys[i].2 = bytes;
    }

    /// Current bytes of a key.
    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.find(k(key).0).map(|i| self.keys[i].2.as_slice())
    }

    fn find(&self, key: u32) -> Option<usize> {
        self.keys.iter().position(|(n, _, _)| n.0 == key)
    }
}

impl Transport for MockSmc {
    fn call(&mut self, input: &KeyData) -> Result<KeyData, SmcError> {
        self.calls.push(input.data8);
        let mut out = KeyData::default();
        match input.data8 {
            CMD_READ_KEYINFO => match self.find(input.key) {
                Some(i) => {
                    let (_, t, b) = &self.keys[i];
                    out.key_info.data_size = b.len() as u32;
                    out.key_info.data_type = t.0;
                }
                None => out.result = RESULT_KEY_NOT_FOUND,
            },
            CMD_READ_BYTES => match self.find(input.key) {
                Some(i) => {
                    let b = &self.keys[i].2;
                    assert_eq!(
                        input.key_info.data_size as usize,
                        b.len(),
                        "wrong requested size"
                    );
                    out.bytes[..b.len()].copy_from_slice(b);
                }
                None => out.result = RESULT_KEY_NOT_FOUND,
            },
            CMD_WRITE_BYTES => match self.find(input.key) {
                Some(i) => {
                    let n = input.key_info.data_size as usize;
                    self.keys[i].2 = input.bytes[..n].to_vec();
                    self.writes.push((self.keys[i].0, self.keys[i].2.clone()));
                }
                None => out.result = RESULT_KEY_NOT_FOUND,
            },
            CMD_READ_INDEX => match self.keys.get(input.data32 as usize) {
                Some((n, _, _)) => out.key = n.0,
                None => out.result = 0x85,
            },
            other => panic!("unexpected command {other}"),
        }
        Ok(out)
    }
}

pub fn fpe2(rpm: u16) -> Vec<u8> {
    (rpm << 2).to_be_bytes().to_vec()
}
pub fn flt(v: f32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}
pub fn sp78(c: f32) -> Vec<u8> {
    ((c * 256.0) as i16).to_be_bytes().to_vec()
}

/// MacBook Pro without T2: 2 fans, `fpe2`, `FS! ` bitmask with fan 1 in manual.
pub fn legacy_mac() -> MockSmc {
    let mut id = vec![0u8; 16];
    id[4..10].copy_from_slice(b"Leftsd");
    MockSmc::new(&[
        ("FNum", "ui8 ", vec![2]),
        ("FS! ", "ui16", vec![0x00, 0x02]),
        ("F0Ac", "fpe2", fpe2(2160)),
        ("F0Mn", "fpe2", fpe2(2000)),
        ("F0Mx", "fpe2", fpe2(6000)),
        ("F0Tg", "fpe2", fpe2(2000)),
        ("F0ID", "{fds", id),
        ("F1Ac", "fpe2", fpe2(3500)),
        ("F1Mn", "fpe2", fpe2(2000)),
        ("F1Mx", "fpe2", fpe2(5500)),
        ("F1Tg", "fpe2", fpe2(3500)),
        ("TC0P", "sp78", sp78(55.5)),
        ("TC0D", "sp78", sp78(61.0)),
        ("TB0T", "sp78", sp78(0.0)), // implausible reading: must be filtered out
        ("LKSB", "{lkb", vec![0, 0]),
    ])
}

/// T2 Mac: 1 fan, `flt `, `F0Md` in manual and `Ftst` present.
pub fn t2_mac() -> MockSmc {
    MockSmc::new(&[
        ("FNum", "ui8 ", vec![1]),
        ("Ftst", "ui8 ", vec![0]),
        ("F0Md", "ui8 ", vec![1]),
        ("F0Ac", "flt ", flt(1800.0)),
        ("F0Mn", "flt ", flt(1200.0)),
        ("F0Mx", "flt ", flt(5200.0)),
        ("F0Tg", "flt ", flt(1800.0)),
        ("TC0P", "flt ", flt(48.25)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FanMode, ModeKey, Smc, Value};

    #[test]
    fn legacy_fan_report() {
        let mut smc = Smc::with_transport(legacy_mac());
        let r = smc.fan_report().unwrap();
        assert_eq!(r.mode_key, ModeKey::Bitmask);
        assert!(!r.has_ftst);
        assert_eq!(r.fans.len(), 2);
        let f0 = &r.fans[0];
        assert_eq!(f0.actual_rpm, Some(2160.0));
        assert_eq!(f0.max_rpm, Some(6000.0));
        assert_eq!(f0.safe_rpm, None);
        assert_eq!(f0.rpm_type, Some(k("fpe2")));
        assert_eq!(f0.name.as_deref(), Some("Leftsd"));
        assert_eq!(f0.mode, FanMode::Auto);
        assert_eq!(r.fans[1].mode, FanMode::Manual);
    }

    #[test]
    fn t2_fan_report() {
        let mut smc = Smc::with_transport(t2_mac());
        let r = smc.fan_report().unwrap();
        assert_eq!(r.mode_key, ModeKey::PerFan);
        assert!(r.has_ftst);
        assert_eq!(r.fans[0].rpm_type, Some(k("flt ")));
        assert_eq!(r.fans[0].min_rpm, Some(1200.0));
        assert_eq!(r.fans[0].mode, FanMode::Manual);
    }

    #[test]
    fn temperatures_filter_and_label() {
        let mut smc = Smc::with_transport(legacy_mac());
        let temps = smc.temperatures().unwrap();
        let keys: Vec<_> = temps.iter().map(|s| s.key.to_string()).collect();
        assert_eq!(keys, vec!["TC0P", "TC0D"]);
        assert_eq!(temps[0].celsius, 55.5);
        assert_eq!(temps[0].label, Some("CPU Proximity"));

        let mut t2 = Smc::with_transport(t2_mac());
        assert_eq!(t2.temperatures().unwrap()[0].celsius, 48.25);
    }

    #[test]
    fn enumerates_all_keys() {
        let mut smc = Smc::with_transport(t2_mac());
        let keys = smc.all_keys().unwrap();
        assert_eq!(keys.len(), 9);
        assert_eq!(keys[0], k("#KEY"));
        assert_eq!(keys[8], k("TC0P"));
    }

    #[test]
    fn missing_key_is_reported() {
        let mut smc = Smc::with_transport(t2_mac());
        assert!(matches!(smc.read(k("ZZZZ")), Err(SmcError::KeyNotFound(_))));
        assert!(!smc.has_key(k("ZZZZ")).unwrap());
    }

    #[test]
    fn key_info_is_cached() {
        let mut smc = Smc::with_transport(t2_mac());
        smc.read(k("F0Ac")).unwrap();
        smc.read(k("F0Ac")).unwrap();
        let t = &smc_transport(&mut smc).calls;
        assert_eq!(t.iter().filter(|&&c| c == CMD_READ_KEYINFO).count(), 1);
        assert_eq!(t.iter().filter(|&&c| c == CMD_READ_BYTES).count(), 2);
    }

    #[test]
    fn write_checks_size_and_roundtrips() {
        let mut smc = Smc::with_transport(legacy_mac());
        assert!(smc.write_raw(k("F0Tg"), &[1, 2, 3]).is_err());
        let info = smc.key_info(k("F0Tg")).unwrap();
        let bytes = crate::value::encode_rpm(&info, 3000.0).unwrap();
        smc.write_raw(k("F0Tg"), &bytes).unwrap();
        assert_eq!(smc.read(k("F0Tg")).unwrap(), Value::Float(3000.0));
    }

    fn smc_transport(smc: &mut Smc<MockSmc>) -> &mut MockSmc {
        smc.transport_mut()
    }

    #[test]
    fn bitmask_manual_toggles_only_its_bit() {
        let mut smc = Smc::with_transport(legacy_mac()); // FS! = 0b10
        smc.set_fan_manual(0, ModeKey::Bitmask, true).unwrap();
        assert_eq!(smc.read(k("FS! ")).unwrap(), Value::Unsigned(0b11));
        smc.set_fan_manual(1, ModeKey::Bitmask, false).unwrap();
        assert_eq!(smc.read(k("FS! ")).unwrap(), Value::Unsigned(0b01));
        smc.set_fan_manual(0, ModeKey::Bitmask, false).unwrap();
        assert_eq!(smc.read(k("FS! ")).unwrap(), Value::Unsigned(0));
    }

    #[test]
    fn per_fan_manual() {
        let mut smc = Smc::with_transport(t2_mac());
        smc.set_fan_manual(0, ModeKey::PerFan, false).unwrap();
        assert_eq!(smc.fan_mode(0, ModeKey::PerFan).unwrap(), FanMode::Auto);
        smc.set_fan_manual(0, ModeKey::PerFan, true).unwrap();
        assert_eq!(smc.fan_mode(0, ModeKey::PerFan).unwrap(), FanMode::Manual);
    }

    #[test]
    fn target_respects_fan_range() {
        let mut smc = Smc::with_transport(legacy_mac()); // F0: 2000..=6000
        assert!(smc.set_fan_target(0, 1999.0).is_err());
        assert!(smc.set_fan_target(0, 6001.0).is_err());
        smc.set_fan_target(0, 3000.0).unwrap();
        assert_eq!(smc.read(k("F0Tg")).unwrap(), Value::Float(3000.0));

        let mut t2 = Smc::with_transport(t2_mac()); // flt, 1200..=5200
        t2.set_fan_target(0, 2500.0).unwrap();
        assert_eq!(t2.read(k("F0Tg")).unwrap(), Value::Float(2500.0));
    }

    #[test]
    fn unknown_mode_key_refuses_write() {
        let mut smc = Smc::with_transport(t2_mac());
        assert!(smc.set_fan_manual(0, ModeKey::Unknown, true).is_err());
    }
}
