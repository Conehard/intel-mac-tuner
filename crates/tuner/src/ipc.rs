//! Control-socket protocol (`paths::SOCKET`): one request line, one reply line.
//!
//! Requests:
//!   `status`
//!   `set auto`
//!   `set fixed <rpm>`
//!   `set sensor <temp_min_c> <temp_max_c>`
//! Replies: `ok …` or `err <message>`.
//!
//! The socket file is `root:admin 0660`: only administrators talk to the daemon.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::paths;

/// Maximum accepted size of a line (request or reply).
pub const MAX_LINE: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Status,
    SetAuto,
    SetFixed {
        rpm: f64,
    },
    SetSensor {
        temp_min_c: f64,
        temp_max_c: f64,
    },
    /// Turns Turbo Boost on (`true`) or off (`false`).
    SetTurbo {
        enabled: bool,
    },
    /// Pauses (`false`) or resumes (`true`) Spotlight indexing (needs root).
    SetSpotlight {
        indexing: bool,
    },
    /// Frees inactive memory now (`purge`). One-shot, stateless.
    PurgeMemory,
    /// Enables/disables an optional Apple LaunchAgent in the `gui/<uid>` domain.
    /// The daemon only accepts labels from its fixed list.
    SetAppleAgent {
        enable: bool,
        uid: u32,
        label: String,
    },
}

fn uint(s: &str) -> Result<u32, String> {
    s.parse::<u32>().map_err(|_| format!("invalid uid: {s:?}"))
}

fn num(s: &str) -> Result<f64, String> {
    s.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("invalid number: {s:?}"))
}

impl Request {
    pub fn parse(line: &str) -> Result<Request, String> {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["status"] => Ok(Request::Status),
            ["set", "auto"] => Ok(Request::SetAuto),
            ["set", "fixed", rpm] => Ok(Request::SetFixed { rpm: num(rpm)? }),
            ["set", "sensor", lo, hi] => Ok(Request::SetSensor {
                temp_min_c: num(lo)?,
                temp_max_c: num(hi)?,
            }),
            ["turbo", "on"] => Ok(Request::SetTurbo { enabled: true }),
            ["turbo", "off"] => Ok(Request::SetTurbo { enabled: false }),
            ["spotlight", "on"] => Ok(Request::SetSpotlight { indexing: true }),
            ["spotlight", "off"] => Ok(Request::SetSpotlight { indexing: false }),
            ["purge"] => Ok(Request::PurgeMemory),
            ["apple-agent", en @ ("enable" | "disable"), uid, label] => {
                Ok(Request::SetAppleAgent {
                    enable: *en == "enable",
                    uid: uint(uid)?,
                    label: (*label).to_string(),
                })
            }
            _ => Err(format!("unknown request: {line:?}")),
        }
    }

    pub fn encode(&self) -> String {
        match self {
            Request::Status => "status".into(),
            Request::SetAuto => "set auto".into(),
            Request::SetFixed { rpm } => format!("set fixed {rpm}"),
            Request::SetSensor {
                temp_min_c,
                temp_max_c,
            } => format!("set sensor {temp_min_c} {temp_max_c}"),
            Request::SetTurbo { enabled } => {
                format!("turbo {}", if *enabled { "on" } else { "off" })
            }
            Request::SetSpotlight { indexing } => {
                format!("spotlight {}", if *indexing { "on" } else { "off" })
            }
            Request::PurgeMemory => "purge".into(),
            Request::SetAppleAgent { enable, uid, label } => format!(
                "apple-agent {} {uid} {label}",
                if *enable { "enable" } else { "disable" }
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanStatus {
    pub index: u8,
    pub rpm: f64,
    pub min: f64,
    pub max: f64,
}

/// Turbo Boost state reported by the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurboStatus {
    /// The daemon does not manage Turbo (turbo.manage = false).
    Unmanaged,
    Enabled,
    Disabled,
    /// Managed, but the state could not be read (kext missing, error).
    Unknown,
}

impl TurboStatus {
    fn code(self) -> &'static str {
        match self {
            TurboStatus::Unmanaged => "unmanaged",
            TurboStatus::Enabled => "on",
            TurboStatus::Disabled => "off",
            TurboStatus::Unknown => "unknown",
        }
    }

    fn from_code(s: &str) -> TurboStatus {
        match s {
            "on" => TurboStatus::Enabled,
            "off" => TurboStatus::Disabled,
            "unmanaged" => TurboStatus::Unmanaged,
            _ => TurboStatus::Unknown,
        }
    }
}

/// Snapshot of the daemon's state.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    /// `auto`, `fixed` or `sensor`.
    pub mode: String,
    pub fixed_rpm: f64,
    pub sensor_key: String,
    pub temp_min_c: f64,
    pub temp_max_c: f64,
    /// Sensor actually read and its temperature.
    pub temp: Option<(String, f64)>,
    pub panic: bool,
    pub turbo: TurboStatus,
    pub fans: Vec<FanStatus>,
}

impl Status {
    /// `ok mode=… fixed=… key=… tmin=… tmax=… temp=KEY:VAL|- panic=0|1 fan=I:RPM:MIN:MAX …`
    pub fn encode(&self) -> String {
        let mut s = format!(
            "ok mode={} fixed={:.0} key={} tmin={} tmax={} temp={} panic={} turbo={}",
            self.mode,
            self.fixed_rpm,
            self.sensor_key,
            self.temp_min_c,
            self.temp_max_c,
            self.temp
                .as_ref()
                .map_or("-".to_string(), |(k, c)| format!("{k}:{c:.1}")),
            u8::from(self.panic),
            self.turbo.code(),
        );
        for f in &self.fans {
            s += &format!(" fan={}:{:.0}:{:.0}:{:.0}", f.index, f.rpm, f.min, f.max);
        }
        s
    }

    pub fn decode(line: &str) -> Result<Status, String> {
        let body = line.strip_prefix("ok ").ok_or_else(|| reply_error(line))?;
        let mut st = Status {
            mode: String::new(),
            fixed_rpm: 0.0,
            sensor_key: String::new(),
            temp_min_c: 0.0,
            temp_max_c: 0.0,
            temp: None,
            panic: false,
            turbo: TurboStatus::Unmanaged,
            fans: Vec::new(),
        };
        for part in body.split_whitespace() {
            let (k, v) = part
                .split_once('=')
                .ok_or_else(|| format!("invalid field: {part:?}"))?;
            match k {
                "mode" => st.mode = v.to_string(),
                "fixed" => st.fixed_rpm = num(v)?,
                "key" => st.sensor_key = v.to_string(),
                "tmin" => st.temp_min_c = num(v)?,
                "tmax" => st.temp_max_c = num(v)?,
                "temp" if v == "-" => st.temp = None,
                "temp" => {
                    let (key, c) = v.split_once(':').ok_or("invalid temp")?;
                    st.temp = Some((key.to_string(), num(c)?));
                }
                "panic" => st.panic = v == "1",
                "turbo" => st.turbo = TurboStatus::from_code(v),
                "fan" => {
                    let p: Vec<&str> = v.split(':').collect();
                    let [i, rpm, min, max] = p.as_slice() else {
                        return Err(format!("invalid fan: {v:?}"));
                    };
                    st.fans.push(FanStatus {
                        index: i.parse().map_err(|_| format!("invalid fan: {v:?}"))?,
                        rpm: num(rpm)?,
                        min: num(min)?,
                        max: num(max)?,
                    });
                }
                _ => {} // new fields from future versions are ignored
            }
        }
        if st.mode.is_empty() {
            return Err("reply has no mode".into());
        }
        Ok(st)
    }
}

fn reply_error(line: &str) -> String {
    line.strip_prefix("err ").unwrap_or(line).to_string()
}

/// Sends a request to the daemon and returns the reply line (without interpreting `ok`/`err`).
pub fn send(req: &Request) -> Result<String, String> {
    let mut stream = UnixStream::connect(paths::SOCKET).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
            "the daemon is not running (sudo tunerctl install)".to_string()
        }
        std::io::ErrorKind::PermissionDenied => {
            "no permission on the socket: your user must be an administrator".to_string()
        }
        _ => format!("could not talk to the daemon: {e}"),
    })?;
    let timeout = Some(Duration::from_secs(3));
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);
    stream
        .write_all(format!("{}\n", req.encode()).as_bytes())
        .map_err(|e| format!("failed to send: {e}"))?;
    let mut line = String::new();
    BufReader::new(stream.take(MAX_LINE as u64))
        .read_line(&mut line)
        .map_err(|e| format!("no reply from the daemon: {e}"))?;
    let line = line.trim_end().to_string();
    if line.is_empty() {
        return Err("empty reply from the daemon".into());
    }
    Ok(line)
}

/// Requests the current state.
pub fn status() -> Result<Status, String> {
    Status::decode(&send(&Request::Status)?)
}

/// Changes the mode; `Ok(())` if the daemon accepted and saved it.
pub fn set(req: &Request) -> Result<(), String> {
    let line = send(req)?;
    if line == "ok" || line.starts_with("ok ") {
        Ok(())
    } else {
        Err(reply_error(&line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_roundtrip() {
        for r in [
            Request::Status,
            Request::SetAuto,
            Request::SetFixed { rpm: 3000.0 },
            Request::SetSensor {
                temp_min_c: 50.0,
                temp_max_c: 85.5,
            },
            Request::SetTurbo { enabled: true },
            Request::SetTurbo { enabled: false },
            Request::SetSpotlight { indexing: true },
            Request::SetSpotlight { indexing: false },
            Request::PurgeMemory,
            Request::SetAppleAgent {
                enable: false,
                uid: 501,
                label: "com.apple.photoanalysisd".into(),
            },
            Request::SetAppleAgent {
                enable: true,
                uid: 501,
                label: "com.apple.assistantd".into(),
            },
        ] {
            assert_eq!(Request::parse(&r.encode()).unwrap(), r);
        }
    }

    #[test]
    fn bad_requests() {
        for line in [
            "",
            "set",
            "set fixed",
            "set fixed x",
            "set sensor 50",
            "rm -rf /",
            "set fixed nan",
            "apple-agent",
            "apple-agent disable",
            "apple-agent disable 501",
            "apple-agent frobnicate 501 com.apple.x",
            "apple-agent disable xxx com.apple.x",
        ] {
            assert!(Request::parse(line).is_err(), "{line:?}");
        }
    }

    #[test]
    fn status_roundtrip() {
        let st = Status {
            mode: "sensor".into(),
            fixed_rpm: 3000.0,
            sensor_key: "TCXC".into(),
            temp_min_c: 50.0,
            temp_max_c: 85.0,
            temp: Some(("TCXC".into(), 62.5)),
            panic: false,
            turbo: TurboStatus::Disabled,
            fans: vec![FanStatus {
                index: 0,
                rpm: 2400.0,
                min: 1200.0,
                max: 7200.0,
            }],
        };
        assert_eq!(Status::decode(&st.encode()).unwrap(), st);

        let none = Status {
            temp: None,
            panic: true,
            fans: vec![],
            ..st
        };
        assert_eq!(Status::decode(&none.encode()).unwrap(), none);
    }

    #[test]
    fn status_errors() {
        assert_eq!(
            Status::decode("err daemon busy").unwrap_err(),
            "daemon busy"
        );
        assert!(Status::decode("ok fan=0:1").is_err());
        assert!(Status::decode("ok panic=0").is_err());
        // unknown fields are tolerated
        assert!(Status::decode("ok mode=auto future=1").is_ok());
    }
}
