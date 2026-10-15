//! Fan control loop. Each `tick` reads the temperature and decides what to write.
//!
//! Safety rules, in priority order:
//! 1. Temperature >= `panic_temp_c`: all fans at maximum until it drops 5 °C.
//! 2. No temperature reading: return control to the SMC (Auto mode).
//! 3. Speed always within `F{n}Mn..=F{n}Mx` (the `smc` crate refuses the rest).

use smc::{keys, FanMode, FourCC, ModeKey, Smc, SmcError, Transport};

use crate::config::{Config, Mode};

/// How much the speed may drop per tick in sensor mode (rising is immediate).
pub const STEP_DOWN_RPM: f64 = 300.0;
/// Minimum difference between the current and desired target to rewrite.
pub const WRITE_DEADBAND_RPM: f64 = 25.0;
/// Margin to leave panic mode.
pub const PANIC_EXIT_MARGIN_C: f64 = 5.0;
/// Minimum target change worth a log line in sensor mode.
const LOG_STEP_RPM: f64 = 500.0;
/// Sensors tried if the configured one does not respond.
const FALLBACK_SENSORS: [&[u8; 4]; 4] = [b"TCXC", b"TC0P", b"TC0D", b"TC0E"];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FanLimits {
    pub index: u8,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Applied {
    /// Nothing applied yet (start, reload or return from sleep).
    Nothing,
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, Default)]
struct CurveState {
    target: Option<f64>,
    /// Temperature that brought the speed to its current value (hysteresis reference).
    temp_ref: f64,
    last_logged: Option<f64>,
}

pub struct Controller {
    cfg: Config,
    mode_key: ModeKey,
    fans: Vec<FanLimits>,
    states: Vec<CurveState>,
    applied: Applied,
    panic: bool,
}

fn round10(v: f64) -> f64 {
    (v / 10.0).round() * 10.0
}

impl Controller {
    /// Reads from the hardware how many fans exist, their limits and the mode key.
    pub fn new<T: Transport>(cfg: Config, smc: &mut Smc<T>) -> Result<Self, SmcError> {
        let mode_key = smc.detect_mode_key()?;
        if mode_key == ModeKey::Unknown {
            return Err(SmcError::InvalidValue(
                "this Mac exposes neither FS! nor F0Md".into(),
            ));
        }
        let count = smc.fan_count()?.min(10);
        let mut fans = Vec::new();
        for index in 0..count {
            let min = smc.read_f64(FourCC::fan(index, keys::SUFFIX_MIN)?)?;
            let max = smc.read_f64(FourCC::fan(index, keys::SUFFIX_MAX)?)?;
            if !(min > 0.0 && max > min) {
                return Err(SmcError::InvalidValue(format!(
                    "fan {index} has odd limits: min {min}, max {max}"
                )));
            }
            fans.push(FanLimits { index, min, max });
        }
        if fans.is_empty() {
            return Err(SmcError::InvalidValue("no fans (FNum = 0)".into()));
        }
        let states = vec![CurveState::default(); fans.len()];
        Ok(Controller {
            cfg,
            mode_key,
            fans,
            states,
            applied: Applied::Nothing,
            panic: false,
        })
    }

    pub fn fans(&self) -> &[FanLimits] {
        &self.fans
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn mode_key(&self) -> ModeKey {
        self.mode_key
    }

    pub fn in_panic(&self) -> bool {
        self.panic
    }

    /// Current state for the control socket.
    pub fn status<T: Transport>(&self, smc: &mut Smc<T>) -> crate::ipc::Status {
        let mode = match self.cfg.mode {
            Mode::Auto => "auto",
            Mode::Fixed => "fixed",
            Mode::Sensor => "sensor",
        };
        let fans = self
            .fans
            .iter()
            .map(|f| crate::ipc::FanStatus {
                index: f.index,
                rpm: FourCC::fan(f.index, keys::SUFFIX_ACTUAL)
                    .and_then(|k| smc.read_f64(k))
                    .unwrap_or(0.0),
                min: f.min,
                max: f.max,
            })
            .collect();
        crate::ipc::Status {
            mode: mode.into(),
            fixed_rpm: self.cfg.fixed_rpm,
            sensor_key: self.cfg.sensor.key.to_string().trim_end().to_string(),
            temp_min_c: self.cfg.sensor.temp_min_c,
            temp_max_c: self.cfg.sensor.temp_max_c,
            temp: self
                .read_temp(smc)
                .map(|(k, c)| (k.to_string().trim_end().to_string(), c)),
            panic: self.panic,
            // Turbo is managed by the daemon, not by the fan controller:
            // the daemon fills this field in afterwards.
            turbo: crate::ipc::TurboStatus::Unmanaged,
            fans,
        }
    }

    /// Swaps the configuration; the next `tick` reapplies everything.
    pub fn set_config(&mut self, cfg: Config) {
        self.cfg = cfg;
        self.reset();
    }

    /// Forgets the applied state (use after waking from sleep).
    pub fn reset(&mut self) {
        self.applied = Applied::Nothing;
        self.states = vec![CurveState::default(); self.fans.len()];
    }

    /// Temperature of the configured sensor, or of the first fallback that responds.
    pub fn read_temp<T: Transport>(&self, smc: &mut Smc<T>) -> Option<(FourCC, f64)> {
        let fallbacks = FALLBACK_SENSORS.iter().map(|b| FourCC::from_bytes(**b));
        for key in std::iter::once(self.cfg.sensor.key).chain(fallbacks) {
            if let Ok(c) = smc.read_f64(key) {
                if c > 0.0 && c < 130.0 {
                    return Some((key, c));
                }
            }
        }
        None
    }

    /// Returns all fans to the SMC. Tries all even if one fails.
    pub fn restore_auto<T: Transport>(&mut self, smc: &mut Smc<T>) -> Result<(), SmcError> {
        let mut first_err = None;
        for fan in &self.fans {
            if let Err(e) = smc.set_fan_manual(fan.index, self.mode_key, false) {
                first_err.get_or_insert(e);
            }
        }
        self.states = vec![CurveState::default(); self.fans.len()];
        match first_err {
            Some(e) => {
                self.applied = Applied::Nothing;
                Err(e)
            }
            None => {
                self.applied = Applied::Auto;
                Ok(())
            }
        }
    }

    /// Speed wanted by the curve, with hysteresis and gradual decrease already applied.
    fn curve_target(&mut self, i: usize, temp: f64) -> f64 {
        let fan = self.fans[i];
        let s = &self.cfg.sensor;
        let lo = s.rpm_min.unwrap_or(fan.min).clamp(fan.min, fan.max);
        let hi = s.rpm_max.unwrap_or(fan.max).clamp(lo, fan.max);
        let frac = ((temp - s.temp_min_c) / (s.temp_max_c - s.temp_min_c)).clamp(0.0, 1.0);
        let raw = round10(lo + frac * (hi - lo)).clamp(fan.min, fan.max);

        let st = &mut self.states[i];
        let next = match st.target {
            None => {
                st.temp_ref = temp;
                raw
            }
            Some(cur) if raw > cur => {
                st.temp_ref = temp;
                raw
            }
            Some(cur) if raw < cur && temp <= st.temp_ref - s.hysteresis_c => {
                raw.max(cur - STEP_DOWN_RPM)
            }
            Some(cur) => cur,
        };
        st.target = Some(next);
        next
    }

    /// Ensures manual mode and target `desired` on fan `i`. Rewrites if the SMC or another program changed something.
    fn ensure_manual<T: Transport>(
        &mut self,
        smc: &mut Smc<T>,
        i: usize,
        desired: f64,
    ) -> Result<(), SmcError> {
        let fan = self.fans[i];
        let desired = desired.clamp(fan.min, fan.max);
        let mode_now = smc.fan_mode(fan.index, self.mode_key)?;
        if mode_now != FanMode::Manual {
            smc.set_fan_manual(fan.index, self.mode_key, true)?;
        }
        let current = smc
            .read_f64(FourCC::fan(fan.index, keys::SUFFIX_TARGET)?)
            .ok();
        let needs_write = mode_now != FanMode::Manual
            || current.map_or(true, |tg| (tg - desired).abs() >= WRITE_DEADBAND_RPM);
        if needs_write {
            smc.set_fan_target(fan.index, desired)?;
        }
        Ok(())
    }

    fn go_auto<T: Transport>(
        &mut self,
        smc: &mut Smc<T>,
        why: &str,
        ev: &mut Vec<String>,
    ) -> Result<(), SmcError> {
        if self.applied != Applied::Auto {
            self.restore_auto(smc)?;
            ev.push(why.to_string());
        }
        Ok(())
    }

    /// One step of the loop. Returns events for the log (only transitions and large changes).
    pub fn tick<T: Transport>(&mut self, smc: &mut Smc<T>) -> Result<Vec<String>, SmcError> {
        let mut ev = Vec::new();
        let temp = self.read_temp(smc);
        let panic_at = self.cfg.panic_temp_c;

        if let Some((key, c)) = temp {
            if !self.panic && c >= panic_at {
                self.panic = true;
                ev.push(format!(
                    "PANIC: {key} = {c:.1} °C (limit {panic_at} °C); fans at maximum"
                ));
            } else if self.panic && c < panic_at - PANIC_EXIT_MARGIN_C {
                self.panic = false;
                self.reset();
                ev.push(format!(
                    "panic over: {key} = {c:.1} °C; returning to the configured mode"
                ));
            }
        }

        if self.panic {
            for i in 0..self.fans.len() {
                let max = self.fans[i].max;
                self.ensure_manual(smc, i, max)?;
            }
            self.applied = Applied::Manual;
            return Ok(ev);
        }

        let Some((key, c)) = temp else {
            if self.cfg.mode != Mode::Auto {
                self.go_auto(
                    smc,
                    "no temperature reading: returning the fans to the SMC",
                    &mut ev,
                )?;
            }
            return Ok(ev);
        };

        match self.cfg.mode {
            Mode::Auto => self.go_auto(smc, "auto mode: SMC controlling the fans", &mut ev)?,
            Mode::Fixed => {
                for i in 0..self.fans.len() {
                    let fan = self.fans[i];
                    let desired = self.cfg.fixed_rpm.clamp(fan.min, fan.max);
                    self.ensure_manual(smc, i, desired)?;
                    if self.applied != Applied::Manual {
                        ev.push(format!("fixed mode: fan {} at {desired:.0} RPM", fan.index));
                    }
                }
                self.applied = Applied::Manual;
            }
            Mode::Sensor => {
                for i in 0..self.fans.len() {
                    let desired = self.curve_target(i, c);
                    self.ensure_manual(smc, i, desired)?;
                    let st = &mut self.states[i];
                    if st
                        .last_logged
                        .map_or(true, |l| (l - desired).abs() >= LOG_STEP_RPM)
                    {
                        st.last_logged = Some(desired);
                        ev.push(format!(
                            "sensor mode: {key} {c:.1} °C -> fan {} at {desired:.0} RPM",
                            self.fans[i].index
                        ));
                    }
                }
                self.applied = Applied::Manual;
            }
        }
        Ok(ev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smc::mock::{fpe2, sp78, MockSmc};
    use smc::Value;

    /// A MacBook Pro like a typical one: 1 fan, fpe2, FS!, range 1200–7200.
    fn mac(temp: f32) -> Smc<MockSmc> {
        Smc::with_transport(MockSmc::new(&[
            ("FNum", "ui8 ", vec![1]),
            ("FS! ", "ui16", vec![0, 0]),
            ("F0Ac", "fpe2", fpe2(1270)),
            ("F0Mn", "fpe2", fpe2(1200)),
            ("F0Mx", "fpe2", fpe2(7200)),
            ("F0Tg", "fpe2", fpe2(1270)),
            ("TCXC", "sp78", sp78(temp)),
        ]))
    }

    fn cfg(text: &str) -> Config {
        Config::parse(text).unwrap()
    }

    fn sensor_cfg() -> Config {
        cfg("[fans]\nmode = \"sensor\"\n[fans.sensor]\nkey = \"TCXC\"\ntemp_min_c = 50\ntemp_max_c = 85\nhysteresis_c = 3\n")
    }

    fn set_temp(smc: &mut Smc<MockSmc>, c: f32) {
        smc.transport_mut().set("TCXC", sp78(c));
    }

    fn target(smc: &mut Smc<MockSmc>) -> f64 {
        smc.read_f64("F0Tg".parse().unwrap()).unwrap()
    }

    fn manual(smc: &mut Smc<MockSmc>) -> bool {
        smc.read("FS! ".parse().unwrap()).unwrap() == Value::Unsigned(1)
    }

    fn writes(smc: &mut Smc<MockSmc>) -> usize {
        smc.transport_mut().writes.len()
    }

    #[test]
    fn reads_fan_limits() {
        let mut smc = mac(55.0);
        let c = Controller::new(Config::default(), &mut smc).unwrap();
        assert_eq!(
            c.fans(),
            &[FanLimits {
                index: 0,
                min: 1200.0,
                max: 7200.0
            }]
        );
        assert_eq!(c.mode_key(), ModeKey::Bitmask);
    }

    #[test]
    fn auto_mode_releases_once() {
        let mut smc = mac(55.0);
        smc.transport_mut().set("FS! ", vec![0, 1]);
        let mut c = Controller::new(Config::default(), &mut smc).unwrap();
        assert_eq!(c.tick(&mut smc).unwrap().len(), 1);
        assert!(!manual(&mut smc));
        let n = writes(&mut smc);
        assert!(c.tick(&mut smc).unwrap().is_empty());
        assert_eq!(
            writes(&mut smc),
            n,
            "should not rewrite in stable auto mode"
        );
    }

    #[test]
    fn fixed_mode_sets_and_reasserts() {
        let mut smc = mac(55.0);
        let mut c = Controller::new(
            cfg("[fans]\nmode = \"fixed\"\nfixed_rpm = 3000\n"),
            &mut smc,
        )
        .unwrap();
        c.tick(&mut smc).unwrap();
        assert!(manual(&mut smc));
        assert_eq!(target(&mut smc), 3000.0);
        let n = writes(&mut smc);
        c.tick(&mut smc).unwrap();
        assert_eq!(writes(&mut smc), n, "nothing changes, nothing is written");

        // The SMC went back to auto on its own (e.g. after sleep): the daemon reapplies.
        smc.transport_mut().set("FS! ", vec![0, 0]);
        smc.transport_mut().set("F0Tg", fpe2(1270));
        c.tick(&mut smc).unwrap();
        assert!(manual(&mut smc));
        assert_eq!(target(&mut smc), 3000.0);
    }

    #[test]
    fn fixed_rpm_is_clamped_to_fan_range() {
        let mut smc = mac(55.0);
        let mut c = Controller::new(
            cfg("[fans]\nmode = \"fixed\"\nfixed_rpm = 9000\n"),
            &mut smc,
        )
        .unwrap();
        c.tick(&mut smc).unwrap();
        assert_eq!(target(&mut smc), 7200.0);
        let mut c =
            Controller::new(cfg("[fans]\nmode = \"fixed\"\nfixed_rpm = 100\n"), &mut smc).unwrap();
        c.tick(&mut smc).unwrap();
        assert_eq!(target(&mut smc), 1200.0);
    }

    #[test]
    fn sensor_curve_endpoints_and_middle() {
        for (temp, rpm) in [
            (40.0, 1200.0),
            (50.0, 1200.0),
            (67.5, 4200.0),
            (85.0, 7200.0),
            (90.0, 7200.0),
        ] {
            let mut smc = mac(temp);
            let mut c = Controller::new(sensor_cfg(), &mut smc).unwrap();
            c.tick(&mut smc).unwrap();
            assert_eq!(target(&mut smc), rpm, "temp {temp}");
            assert!(manual(&mut smc));
        }
    }

    #[test]
    fn sensor_respects_custom_rpm_range() {
        let mut smc = mac(85.0);
        let text = "[fans]\nmode = \"sensor\"\n[fans.sensor]\nrpm_min = 2000\nrpm_max = 5000\n";
        let mut c = Controller::new(cfg(text), &mut smc).unwrap();
        c.tick(&mut smc).unwrap();
        assert_eq!(target(&mut smc), 5000.0);
        set_temp(&mut smc, 40.0);
        for _ in 0..20 {
            c.tick(&mut smc).unwrap();
        }
        assert_eq!(target(&mut smc), 2000.0);
    }

    #[test]
    fn sensor_goes_up_immediately_and_down_with_hysteresis() {
        let mut smc = mac(60.0);
        let mut c = Controller::new(sensor_cfg(), &mut smc).unwrap();
        c.tick(&mut smc).unwrap();
        let at60 = target(&mut smc);

        set_temp(&mut smc, 75.0);
        c.tick(&mut smc).unwrap();
        let at75 = target(&mut smc);
        assert!(at75 > at60, "rising is immediate");

        set_temp(&mut smc, 73.0); // dropped only 2 °C (< hysteresis of 3)
        c.tick(&mut smc).unwrap();
        assert_eq!(target(&mut smc), at75, "holds within the hysteresis");

        set_temp(&mut smc, 60.0); // dropped a lot
        c.tick(&mut smc).unwrap();
        assert_eq!(
            target(&mut smc),
            at75 - STEP_DOWN_RPM,
            "drops at most STEP_DOWN per tick"
        );
        for _ in 0..30 {
            c.tick(&mut smc).unwrap();
        }
        assert_eq!(target(&mut smc), at60, "reaches the curve value");
    }

    #[test]
    fn panic_forces_max_until_margin() {
        let mut smc = mac(60.0);
        let mut c = Controller::new(
            cfg("[fans]\nmode = \"fixed\"\nfixed_rpm = 2000\n"),
            &mut smc,
        )
        .unwrap();
        c.tick(&mut smc).unwrap();
        assert_eq!(target(&mut smc), 2000.0);

        set_temp(&mut smc, 96.0);
        let ev = c.tick(&mut smc).unwrap();
        assert!(ev[0].starts_with("PANIC"));
        assert_eq!(target(&mut smc), 7200.0);

        set_temp(&mut smc, 92.0); // still above 95 - 5
        c.tick(&mut smc).unwrap();
        assert_eq!(target(&mut smc), 7200.0);

        set_temp(&mut smc, 89.0);
        let ev = c.tick(&mut smc).unwrap();
        assert!(ev[0].starts_with("panic over"));
        assert_eq!(target(&mut smc), 2000.0);
    }

    #[test]
    fn panic_also_applies_in_auto_mode() {
        let mut smc = mac(97.0);
        let mut c = Controller::new(Config::default(), &mut smc).unwrap();
        c.tick(&mut smc).unwrap();
        assert!(manual(&mut smc));
        assert_eq!(target(&mut smc), 7200.0);
        set_temp(&mut smc, 80.0);
        c.tick(&mut smc).unwrap();
        assert!(!manual(&mut smc), "returns to the SMC after panic");
    }

    #[test]
    fn no_temperature_returns_control_to_smc() {
        let mut smc = mac(60.0);
        let mut c = Controller::new(sensor_cfg(), &mut smc).unwrap();
        c.tick(&mut smc).unwrap();
        assert!(manual(&mut smc));
        set_temp(&mut smc, 0.0); // implausible reading on all sensors
        let ev = c.tick(&mut smc).unwrap();
        assert!(ev[0].contains("no temperature reading"));
        assert!(!manual(&mut smc));
    }

    #[test]
    fn falls_back_to_other_sensor() {
        let mut smc = Smc::with_transport(MockSmc::new(&[
            ("FNum", "ui8 ", vec![1]),
            ("FS! ", "ui16", vec![0, 0]),
            ("F0Mn", "fpe2", fpe2(1200)),
            ("F0Mx", "fpe2", fpe2(7200)),
            ("F0Tg", "fpe2", fpe2(1200)),
            ("TC0P", "sp78", sp78(70.0)),
        ]));
        let c = Controller::new(sensor_cfg(), &mut smc).unwrap();
        let (key, temp) = c.read_temp(&mut smc).unwrap();
        assert_eq!((key.to_string().as_str(), temp), ("TC0P", 70.0));
    }

    #[test]
    fn restore_and_reset() {
        let mut smc = mac(60.0);
        let mut c = Controller::new(cfg("[fans]\nmode = \"fixed\"\n"), &mut smc).unwrap();
        c.tick(&mut smc).unwrap();
        c.restore_auto(&mut smc).unwrap();
        assert!(!manual(&mut smc));
        c.reset();
        c.tick(&mut smc).unwrap();
        assert!(manual(&mut smc), "after reset the next tick reapplies");
    }

    #[test]
    fn reload_switches_mode() {
        let mut smc = mac(60.0);
        let mut c = Controller::new(cfg("[fans]\nmode = \"fixed\"\n"), &mut smc).unwrap();
        c.tick(&mut smc).unwrap();
        assert!(manual(&mut smc));
        c.set_config(Config::default());
        c.tick(&mut smc).unwrap();
        assert!(!manual(&mut smc));
    }
}
