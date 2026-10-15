//! The daemon's `config.toml`: fan mode, per-sensor curve and safety limits.

use std::path::Path;

use smc::FourCC;

use crate::toml_lite::{self, TomlValue};

/// File written by `tunerctl install` when it does not exist yet.
pub const DEFAULT_CONFIG: &str = r#"# Intel Mac Tuner — daemon configuration
# After editing: sudo tunerctl reload

[fans]
# auto   = the SMC controls (factory default)
# fixed  = fixed speed (fixed_rpm)
# sensor = temperature -> speed curve (the [fans.sensor] section)
mode = "auto"
fixed_rpm = 3000

[fans.sensor]
# Sensor used for the curve. TCXC = CPU PECI; TC0P = CPU Proximity.
key = "TCXC"
# Below temp_min_c: minimum speed. Above temp_max_c: maximum speed.
temp_min_c = 50
temp_max_c = 85
# Speed range of the curve. Without these lines, the fan's own min and max are used.
# rpm_min = 1200
# rpm_max = 7200
# The speed only drops once the temperature falls at least this much below
# the temperature that made the speed rise (avoids oscillation).
hysteresis_c = 3

[daemon]
# Interval between readings, in milliseconds.
interval_ms = 1000

[safety]
# Above this, fans at maximum until the temperature drops 5 °C.
panic_temp_c = 95

[turbo]
# Should the daemon control Turbo Boost? Requires the DisableTurbo kext installed
# (sudo tunerctl install-turbo). Leave it false to not touch Turbo
# (e.g. if another utility already handles it).
manage = false
# Turbo Boost state at power-on (when manage = true):
# true = on (factory default), false = off.
enabled_on_boot = true
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Fixed,
    Sensor,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SensorCurve {
    pub key: FourCC,
    pub temp_min_c: f64,
    pub temp_max_c: f64,
    pub rpm_min: Option<f64>,
    pub rpm_max: Option<f64>,
    pub hysteresis_c: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurboCfg {
    /// Does the daemon manage Turbo Boost (loading/unloading the kext)?
    pub manage: bool,
    /// Desired Turbo state at startup: true = on.
    pub enabled_on_boot: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub mode: Mode,
    pub fixed_rpm: f64,
    pub sensor: SensorCurve,
    pub interval_ms: u64,
    pub panic_temp_c: f64,
    pub turbo: TurboCfg,
}

impl Default for Config {
    fn default() -> Self {
        // DEFAULT_CONFIG is tested to be valid.
        Config::parse(DEFAULT_CONFIG).expect("invalid DEFAULT_CONFIG")
    }
}

const KNOWN_KEYS: &[&str] = &[
    "fans.mode",
    "fans.fixed_rpm",
    "fans.sensor.key",
    "fans.sensor.temp_min_c",
    "fans.sensor.temp_max_c",
    "fans.sensor.rpm_min",
    "fans.sensor.rpm_max",
    "fans.sensor.hysteresis_c",
    "daemon.interval_ms",
    "safety.panic_temp_c",
    "turbo.manage",
    "turbo.enabled_on_boot",
];

struct Reader(toml_lite::Table);

impl Reader {
    fn num(&self, key: &str) -> Result<Option<f64>, String> {
        match self.0.get(key) {
            None => Ok(None),
            Some(TomlValue::Num(n)) => Ok(Some(*n)),
            Some(other) => Err(format!("{key} must be a number, not {other:?}")),
        }
    }

    fn num_or(&self, key: &str, default: f64) -> Result<f64, String> {
        Ok(self.num(key)?.unwrap_or(default))
    }

    fn str_or<'a>(&'a self, key: &str, default: &'a str) -> Result<&'a str, String> {
        match self.0.get(key) {
            None => Ok(default),
            Some(TomlValue::Str(s)) => Ok(s),
            Some(other) => Err(format!("{key} must be a quoted string, not {other:?}")),
        }
    }

    fn bool_or(&self, key: &str, default: bool) -> Result<bool, String> {
        match self.0.get(key) {
            None => Ok(default),
            Some(TomlValue::Bool(b)) => Ok(*b),
            Some(other) => Err(format!("{key} must be true or false, not {other:?}")),
        }
    }
}

fn check_range(name: &str, v: f64, lo: f64, hi: f64) -> Result<f64, String> {
    if (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(format!(
            "{name} = {v} out of the allowed range ({lo} to {hi})"
        ))
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let table = toml_lite::parse(text)?;
        if let Some(unknown) = table.keys().find(|k| !KNOWN_KEYS.contains(&k.as_str())) {
            return Err(format!("unknown key: {unknown}"));
        }
        let r = Reader(table);

        let mode = match r.str_or("fans.mode", "auto")? {
            "auto" => Mode::Auto,
            "fixed" => Mode::Fixed,
            "sensor" => Mode::Sensor,
            other => {
                return Err(format!(
                    "fans.mode = {other:?}: use \"auto\", \"fixed\" or \"sensor\""
                ))
            }
        };
        let fixed_rpm = check_range(
            "fans.fixed_rpm",
            r.num_or("fans.fixed_rpm", 3000.0)?,
            0.0,
            10000.0,
        )?;

        let key_txt = r.str_or("fans.sensor.key", "TCXC")?;
        let key: FourCC = key_txt
            .parse()
            .map_err(|_| format!("fans.sensor.key = {key_txt:?} is not an SMC key"))?;
        if key.to_bytes()[0] != b'T' {
            return Err(format!(
                "fans.sensor.key = {key_txt:?}: temperature sensors start with T"
            ));
        }
        let temp_min_c = check_range(
            "fans.sensor.temp_min_c",
            r.num_or("fans.sensor.temp_min_c", 50.0)?,
            20.0,
            95.0,
        )?;
        let temp_max_c = check_range(
            "fans.sensor.temp_max_c",
            r.num_or("fans.sensor.temp_max_c", 85.0)?,
            25.0,
            100.0,
        )?;
        if temp_max_c - temp_min_c < 5.0 {
            return Err(format!(
                "fans.sensor: temp_max_c ({temp_max_c}) must be at least 5 °C higher than temp_min_c ({temp_min_c})"
            ));
        }
        let rpm_min = r.num("fans.sensor.rpm_min")?;
        let rpm_max = r.num("fans.sensor.rpm_max")?;
        for (name, v) in [
            ("fans.sensor.rpm_min", rpm_min),
            ("fans.sensor.rpm_max", rpm_max),
        ] {
            if let Some(v) = v {
                check_range(name, v, 0.0, 10000.0)?;
            }
        }
        if let (Some(lo), Some(hi)) = (rpm_min, rpm_max) {
            if lo > hi {
                return Err(format!(
                    "fans.sensor: rpm_min ({lo}) greater than rpm_max ({hi})"
                ));
            }
        }
        let hysteresis_c = check_range(
            "fans.sensor.hysteresis_c",
            r.num_or("fans.sensor.hysteresis_c", 3.0)?,
            0.0,
            20.0,
        )?;
        let interval_ms = check_range(
            "daemon.interval_ms",
            r.num_or("daemon.interval_ms", 1000.0)?,
            250.0,
            10000.0,
        )? as u64;
        let panic_temp_c = check_range(
            "safety.panic_temp_c",
            r.num_or("safety.panic_temp_c", 95.0)?,
            70.0,
            105.0,
        )?;
        if mode == Mode::Sensor && temp_max_c >= panic_temp_c {
            return Err(format!(
                "fans.sensor.temp_max_c ({temp_max_c}) must be lower than safety.panic_temp_c ({panic_temp_c})"
            ));
        }

        let turbo = TurboCfg {
            manage: r.bool_or("turbo.manage", false)?,
            enabled_on_boot: r.bool_or("turbo.enabled_on_boot", true)?,
        };

        Ok(Config {
            mode,
            fixed_rpm,
            sensor: SensorCurve {
                key,
                temp_min_c,
                temp_max_c,
                rpm_min,
                rpm_max,
                hysteresis_c,
            },
            interval_ms,
            panic_temp_c,
            turbo,
        })
    }

    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Generates the full `config.toml` (with the default comments) for these values.
    pub fn to_toml(&self) -> String {
        let mode = match self.mode {
            Mode::Auto => "auto",
            Mode::Fixed => "fixed",
            Mode::Sensor => "sensor",
        };
        let s = &self.sensor;
        let opt = |name: &str, v: Option<f64>, example: f64| match v {
            Some(v) => format!("{name} = {v}"),
            None => format!("# {name} = {example}"),
        };
        format!(
            r#"# Intel Mac Tuner — daemon configuration
# After editing: sudo tunerctl reload (or use the menu bar / tunerctl mode)

[fans]
# auto   = the SMC controls (factory default)
# fixed  = fixed speed (fixed_rpm)
# sensor = temperature -> speed curve (the [fans.sensor] section)
mode = "{mode}"
fixed_rpm = {fixed}

[fans.sensor]
# Sensor used for the curve. TCXC = CPU PECI; TC0P = CPU Proximity.
key = "{key}"
# Below temp_min_c: minimum speed. Above temp_max_c: maximum speed.
temp_min_c = {tmin}
temp_max_c = {tmax}
# Speed range of the curve. Without these lines, the fan's own min and max are used.
{rmin}
{rmax}
# The speed only drops once the temperature falls at least this much below
# the temperature that made the speed rise (avoids oscillation).
hysteresis_c = {hyst}

[daemon]
# Interval between readings, in milliseconds.
interval_ms = {interval}

[safety]
# Above this, fans at maximum until the temperature drops 5 °C.
panic_temp_c = {panic}

[turbo]
# Should the daemon control Turbo Boost? Requires the DisableTurbo kext installed
# (sudo tunerctl install-turbo). Leave it false to not touch Turbo.
manage = {manage}
# Turbo state at power-on (when manage = true): true = on.
enabled_on_boot = {turbo_boot}
"#,
            fixed = self.fixed_rpm,
            key = s.key.to_string().trim_end(),
            tmin = s.temp_min_c,
            tmax = s.temp_max_c,
            rmin = opt("rpm_min", s.rpm_min, 1200.0),
            rmax = opt("rpm_max", s.rpm_max, 7200.0),
            hyst = s.hysteresis_c,
            interval = self.interval_ms,
            panic = self.panic_temp_c,
            manage = self.turbo.manage,
            turbo_boot = self.turbo.enabled_on_boot,
        )
    }

    /// Writes atomically (temp file + rename), permission 0644.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;
        let text = self.to_toml();
        // Never write something that would not pass validation.
        Config::parse(&text).map_err(|e| format!("generated configuration is invalid: {e}"))?;
        let tmp = path.with_extension("toml.tmp");
        let mut f =
            std::fs::File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
        f.write_all(text.as_bytes())
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        f.sync_all()
            .map_err(|e| format!("sync {}: {e}", tmp.display()))?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))
            .map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", path.display()))
    }

    /// Copy with the mode changed, validated as if it came from the file.
    pub fn with_request(&self, req: &crate::ipc::Request) -> Result<Config, String> {
        use crate::ipc::Request;
        let mut c = self.clone();
        match *req {
            Request::Status => {}
            Request::SetAuto => c.mode = Mode::Auto,
            Request::SetFixed { rpm } => {
                c.mode = Mode::Fixed;
                c.fixed_rpm = rpm;
            }
            Request::SetSensor {
                temp_min_c,
                temp_max_c,
            } => {
                c.mode = Mode::Sensor;
                c.sensor.temp_min_c = temp_min_c;
                c.sensor.temp_max_c = temp_max_c;
            }
            Request::SetTurbo { enabled } => {
                // Changing Turbo implies managing it, and the new state becomes the boot default.
                c.turbo.manage = true;
                c.turbo.enabled_on_boot = enabled;
            }
            // System operations (Spotlight, purge, Apple agents) have no state in
            // the config; the daemon runs them directly, without going through here.
            Request::SetSpotlight { .. } | Request::PurgeMemory | Request::SetAppleAgent { .. } => {
            }
        }
        Config::parse(&c.to_toml())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_and_safe() {
        let c = Config::default();
        assert_eq!(c.mode, Mode::Auto);
        assert_eq!(c.sensor.key.to_string(), "TCXC");
        assert_eq!(c.sensor.rpm_min, None);
        assert_eq!(c.interval_ms, 1000);
        assert_eq!(c.panic_temp_c, 95.0);
    }

    #[test]
    fn to_toml_roundtrips() {
        let mut c = Config::default();
        assert_eq!(Config::parse(&c.to_toml()).unwrap(), c);
        c.mode = Mode::Sensor;
        c.sensor.key = "TC0P".parse().unwrap();
        c.sensor.rpm_min = Some(1500.0);
        c.sensor.temp_min_c = 47.5;
        assert_eq!(Config::parse(&c.to_toml()).unwrap(), c);
    }

    #[test]
    fn requests_change_mode_and_are_validated() {
        use crate::ipc::Request;
        let c = Config::default();
        let f = c.with_request(&Request::SetFixed { rpm: 4000.0 }).unwrap();
        assert_eq!((f.mode, f.fixed_rpm), (Mode::Fixed, 4000.0));
        let s = f
            .with_request(&Request::SetSensor {
                temp_min_c: 45.0,
                temp_max_c: 80.0,
            })
            .unwrap();
        assert_eq!(
            (s.mode, s.sensor.temp_min_c, s.sensor.temp_max_c),
            (Mode::Sensor, 45.0, 80.0)
        );
        assert_eq!(s.fixed_rpm, 4000.0, "keeps the rest");
        assert_eq!(s.with_request(&Request::SetAuto).unwrap().mode, Mode::Auto);
        // unsafe values are refused by the same validation as the file
        assert!(c
            .with_request(&Request::SetSensor {
                temp_min_c: 50.0,
                temp_max_c: 99.0
            })
            .is_err());
        assert!(c.with_request(&Request::SetFixed { rpm: 20000.0 }).is_err());
    }

    #[test]
    fn save_is_atomic_and_readable() {
        let dir = std::env::temp_dir().join(format!("imt-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let c = Config {
            mode: Mode::Fixed,
            ..Default::default()
        };
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);
        assert!(!dir.join("config.toml.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_sensor_mode() {
        let c = Config::parse(
            "[fans]\nmode = \"sensor\"\n[fans.sensor]\nkey = \"TC0P\"\ntemp_min_c = 45\ntemp_max_c = 80\nrpm_min = 1500\n",
        )
        .unwrap();
        assert_eq!(c.mode, Mode::Sensor);
        assert_eq!(c.sensor.key.to_string(), "TC0P");
        assert_eq!(c.sensor.rpm_min, Some(1500.0));
        assert_eq!(c.sensor.rpm_max, None);
    }

    #[test]
    fn empty_file_means_defaults() {
        let c = Config::parse("").unwrap();
        assert_eq!(c.mode, Mode::Auto);
        assert_eq!(c.fixed_rpm, 3000.0);
    }

    #[test]
    fn rejects_unsafe_or_wrong_values() {
        let bad = [
            "[fans]\nmode = \"turbo\"",
            "[fans]\nspeed = \"auto\"",
            "[fans.sensor]\nkey = \"F0Ac\"",
            "[fans.sensor]\ntemp_min_c = 80\ntemp_max_c = 82",
            "[fans.sensor]\nrpm_min = 5000\nrpm_max = 2000",
            "[daemon]\ninterval_ms = 10",
            "[safety]\npanic_temp_c = 120",
            "[fans]\nmode = \"sensor\"\n[fans.sensor]\ntemp_max_c = 96\n[safety]\npanic_temp_c = 95",
            "[fans]\nfixed_rpm = \"3000\"",
        ];
        for text in bad {
            assert!(Config::parse(text).is_err(), "should reject: {text}");
        }
    }
}
