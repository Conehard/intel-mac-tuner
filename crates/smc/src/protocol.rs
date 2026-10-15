//! AppleSMC protocol via IOKit:
//! `IOConnectCallStructMethod(conn, 2, &in, 80, &out, &80)`, with the command in
//! byte 42 of the struct (9 = key info, 5 = read, 6 = write, 8 = key by index).
//!
//! The struct layout and the key-info/read/write call flow follow the SMC tool by
//! devnull (Copyright (C) 2006) and Michael Wilber (Portions Copyright (C) 2013),
//! distributed with smcFanControl under the GNU General Public License,
//! version 2 or (at your option) any later version.

use std::collections::HashMap;

use crate::value::{decode, KeyInfo, Value};
use crate::{FourCC, SmcError};

/// `AppleSMC` user-client method selector (`KERNEL_INDEX_SMC`).
pub const KERNEL_INDEX_SMC: u32 = 2;

pub const CMD_READ_BYTES: u8 = 5;
pub const CMD_WRITE_BYTES: u8 = 6;
pub const CMD_READ_INDEX: u8 = 8;
pub const CMD_READ_KEYINFO: u8 = 9;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyDataVers {
    pub major: u8,
    pub minor: u8,
    pub build: u8,
    pub reserved: u8,
    pub release: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyDataPLimit {
    pub version: u16,
    pub length: u16,
    pub cpu_p_limit: u32,
    pub gpu_p_limit: u32,
    pub mem_p_limit: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyDataKeyInfo {
    pub data_size: u32,
    pub data_type: u32,
    pub data_attributes: u8,
}

/// Exact mirror of `SMCKeyData_t` (80 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyData {
    pub key: u32,
    pub vers: KeyDataVers,
    pub p_limit_data: KeyDataPLimit,
    pub key_info: KeyDataKeyInfo,
    pub result: u8,
    pub status: u8,
    pub data8: u8,
    pub data32: u32,
    pub bytes: [u8; 32],
}

// The layout must match the kernel's; the struct is 0x50 bytes with the command at offset 42.
const _: () = assert!(std::mem::size_of::<KeyData>() == 80);
const _: () = assert!(std::mem::offset_of!(KeyData, data8) == 42);
const _: () = assert!(std::mem::offset_of!(KeyData, bytes) == 48);

/// Result codes returned by the SMC in the `result` field.
pub const RESULT_SUCCESS: u8 = 0x00;
pub const RESULT_KEY_NOT_FOUND: u8 = 0x84;

/// Something that can exchange a `KeyData` with the SMC. On macOS it is IOKit; in tests, a mock.
pub trait Transport {
    fn call(&mut self, input: &KeyData) -> Result<KeyData, SmcError>;
}

/// SMC client over any transport, with a `KeyInfo` cache.
pub struct Smc<T: Transport> {
    transport: T,
    info_cache: HashMap<FourCC, KeyInfo>,
}

impl<T: Transport> Smc<T> {
    pub fn with_transport(transport: T) -> Self {
        Smc {
            transport,
            info_cache: HashMap::new(),
        }
    }

    /// Access to the transport (used in tests).
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    fn call(&mut self, input: &KeyData, key: FourCC) -> Result<KeyData, SmcError> {
        let out = self.transport.call(input)?;
        match out.result {
            RESULT_SUCCESS => Ok(out),
            RESULT_KEY_NOT_FOUND => Err(SmcError::KeyNotFound(key)),
            code => Err(SmcError::Smc { key, code }),
        }
    }

    /// Type, size and attributes of the key.
    pub fn key_info(&mut self, key: FourCC) -> Result<KeyInfo, SmcError> {
        if let Some(info) = self.info_cache.get(&key) {
            return Ok(*info);
        }
        let input = KeyData {
            key: key.0,
            data8: CMD_READ_KEYINFO,
            ..Default::default()
        };
        let out = self.call(&input, key)?;
        let info = KeyInfo {
            size: out.key_info.data_size,
            data_type: FourCC(out.key_info.data_type),
            attributes: out.key_info.data_attributes,
        };
        if info.size as usize > 32 {
            return Err(SmcError::InvalidValue(format!(
                "{key}: size {} > 32",
                info.size
            )));
        }
        self.info_cache.insert(key, info);
        Ok(info)
    }

    /// Does the key exist on this Mac?
    pub fn has_key(&mut self, key: FourCC) -> Result<bool, SmcError> {
        match self.key_info(key) {
            Ok(_) => Ok(true),
            Err(SmcError::KeyNotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Raw bytes of the key.
    pub fn read_raw(&mut self, key: FourCC) -> Result<(KeyInfo, Vec<u8>), SmcError> {
        let info = self.key_info(key)?;
        let mut input = KeyData {
            key: key.0,
            data8: CMD_READ_BYTES,
            ..Default::default()
        };
        input.key_info.data_size = info.size;
        let out = self.call(&input, key)?;
        Ok((info, out.bytes[..info.size as usize].to_vec()))
    }

    /// Decoded value of the key.
    pub fn read(&mut self, key: FourCC) -> Result<Value, SmcError> {
        let (info, bytes) = self.read_raw(key)?;
        Ok(decode(info.data_type, &bytes))
    }

    /// Numeric value of the key (error if the type is not numeric).
    pub fn read_f64(&mut self, key: FourCC) -> Result<f64, SmcError> {
        let (info, bytes) = self.read_raw(key)?;
        decode(info.data_type, &bytes)
            .as_f64()
            .ok_or(SmcError::UnsupportedType(info.data_type))
    }

    /// Write raw bytes. Requires root.
    pub fn write_raw(&mut self, key: FourCC, bytes: &[u8]) -> Result<(), SmcError> {
        let info = self.key_info(key)?;
        if bytes.len() != info.size as usize {
            return Err(SmcError::InvalidValue(format!(
                "{key} expects {} byte(s), got {}",
                info.size,
                bytes.len()
            )));
        }
        let mut input = KeyData {
            key: key.0,
            data8: CMD_WRITE_BYTES,
            ..Default::default()
        };
        input.key_info.data_size = info.size;
        input.bytes[..bytes.len()].copy_from_slice(bytes);
        self.call(&input, key).map(|_| ())
    }

    /// Total number of keys (`#KEY`).
    pub fn key_count(&mut self) -> Result<u32, SmcError> {
        let n = self.read_f64(crate::keys::KEY_COUNT)?;
        Ok(n as u32)
    }

    /// Name of the key at position `index` (0-based).
    pub fn key_at(&mut self, index: u32) -> Result<FourCC, SmcError> {
        let input = KeyData {
            data8: CMD_READ_INDEX,
            data32: index,
            ..Default::default()
        };
        let out = self.call(&input, FourCC(0))?;
        Ok(FourCC(out.key))
    }

    /// All SMC keys, in controller order.
    pub fn all_keys(&mut self) -> Result<Vec<FourCC>, SmcError> {
        let n = self.key_count()?;
        (0..n).map(|i| self.key_at(i)).collect()
    }
}
