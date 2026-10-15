//! Apple SMC access on Intel Macs.
//!
//! ```no_run
//! let mut smc = smc::open()?;
//! let report = smc.fan_report()?;
//! println!("{} fan(s), mode via {:?}", report.fans.len(), report.mode_key);
//! # Ok::<(), smc::SmcError>(())
//! ```

mod fans;
mod key;
pub mod protocol;
pub mod value;

#[cfg(any(target_os = "macos", feature = "check-ffi"))]
pub mod iokit;

#[cfg(any(test, feature = "testing"))]
pub mod mock;

pub use fans::{FanInfo, FanMode, FanReport, ModeKey, Sensor};
pub use key::{keys, known_sensor_label, FourCC};
pub use protocol::{Smc, Transport};
pub use value::{KeyInfo, Value};

use std::fmt;

#[derive(Debug)]
pub enum SmcError {
    /// This system has no `AppleSMC` (or is not macOS).
    NoSmc,
    /// An IOKit/Mach call failed.
    Kernel {
        call: &'static str,
        code: i32,
    },
    /// The SMC replied with an error.
    Smc {
        key: FourCC,
        code: u8,
    },
    KeyNotFound(FourCC),
    InvalidKey(String),
    InvalidValue(String),
    UnsupportedType(FourCC),
}

impl fmt::Display for SmcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmcError::NoSmc => write!(
                f,
                "AppleSMC not found (this program only runs on Intel Macs)"
            ),
            SmcError::Kernel { call, code } => {
                let hint = if *code == 0xe00002c1u32 as i32 {
                    " (not privileged: run with sudo)"
                } else {
                    ""
                };
                write!(f, "{call} failed with 0x{:08x}{hint}", *code as u32)
            }
            SmcError::Smc { key, code } => write!(f, "SMC returned error 0x{code:02x} for {key}"),
            SmcError::KeyNotFound(k) => write!(f, "key {k} does not exist on this Mac"),
            SmcError::InvalidKey(k) => {
                write!(f, "invalid key: {k:?} (use 1 to 4 ASCII characters)")
            }
            SmcError::InvalidValue(m) => write!(f, "invalid value: {m}"),
            SmcError::UnsupportedType(t) => write!(f, "unsupported data type: {t}"),
        }
    }
}

impl std::error::Error for SmcError {}

/// SMC client bound to IOKit.
#[cfg(target_os = "macos")]
pub type SystemSmc = Smc<iokit::IoKitTransport>;

/// Open this Mac's SMC.
#[cfg(target_os = "macos")]
pub fn open() -> Result<SystemSmc, SmcError> {
    Ok(Smc::with_transport(iokit::IoKitTransport::open()?))
}

/// Outside macOS there is no SMC.
#[cfg(not(target_os = "macos"))]
pub fn open() -> Result<Smc<Unsupported>, SmcError> {
    Err(SmcError::NoSmc)
}

/// Empty transport used outside macOS.
#[cfg(not(target_os = "macos"))]
pub struct Unsupported;

#[cfg(not(target_os = "macos"))]
impl Transport for Unsupported {
    fn call(&mut self, _input: &protocol::KeyData) -> Result<protocol::KeyData, SmcError> {
        Err(SmcError::NoSmc)
    }
}
