//! `tunerctl fan test`: the first SMC write, temporary and with guaranteed restore.
//!
//! Puts a fan in manual, writes the target RPM, tracks the real RPM and,
//! at the end (or on Ctrl+C, SIGTERM, error or panic), returns the fan to its
//! original state.

use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use smc::{keys, FanMode, FourCC, Smc, SmcError, Transport};

/// Above this the test aborts and returns control to the SMC.
const PANIC_TEMP_C: f64 = 95.0;
const MAX_SECONDS: u64 = 120;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: c_int) {
    STOP.store(true, Ordering::SeqCst);
}

extern "C" {
    fn signal(sig: c_int, handler: extern "C" fn(c_int)) -> usize;
    fn geteuid() -> u32;
}

const SIGHUP: c_int = 1;
const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;

pub struct Options {
    pub index: u8,
    pub rpm: f64,
    pub seconds: u64,
}

impl Options {
    /// `fan test <N> --rpm <RPM> [--seconds <S>]`
    pub fn parse(index: &str, rest: &[&str]) -> Result<Self, String> {
        let index: u8 = index
            .parse()
            .map_err(|_| format!("invalid fan: {index:?}"))?;
        let mut rpm = None;
        let mut seconds = 15;
        let mut it = rest.iter();
        while let Some(&flag) = it.next() {
            let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
            match flag {
                "--rpm" => {
                    rpm = Some(
                        value
                            .parse::<f64>()
                            .map_err(|_| format!("invalid RPM: {value:?}"))?,
                    )
                }
                "--seconds" => {
                    seconds = value
                        .parse()
                        .map_err(|_| format!("invalid seconds: {value:?}"))?
                }
                other => return Err(format!("unknown option: {other}")),
            }
        }
        let rpm = rpm.ok_or("missing --rpm")?;
        if seconds == 0 || seconds > MAX_SECONDS {
            return Err(format!("--seconds must be between 1 and {MAX_SECONDS}"));
        }
        Ok(Options {
            index,
            rpm,
            seconds,
        })
    }
}

/// The fan's previous state, to restore exactly what was there.
struct Original {
    mode: FanMode,
    target: Option<f64>,
}

/// Returns the fan to its original state when it goes out of scope (including on panic).
struct RestoreGuard<'a, T: Transport> {
    smc: &'a mut Smc<T>,
    index: u8,
    mode_key: smc::ModeKey,
    original: Original,
    done: bool,
}

impl<T: Transport> RestoreGuard<'_, T> {
    fn restore(&mut self) -> Result<(), SmcError> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        if self.original.mode == FanMode::Manual {
            if let Some(t) = self.original.target {
                self.smc.set_fan_target(self.index, t)?;
            }
            Ok(())
        } else {
            self.smc.set_fan_manual(self.index, self.mode_key, false)
        }
    }
}

impl<T: Transport> Drop for RestoreGuard<'_, T> {
    fn drop(&mut self) {
        if !self.done {
            match self.restore() {
                Ok(()) => eprintln!("fan {} returned to its original state", self.index),
                Err(e) => eprintln!(
                    "WARNING: failed to restore fan {}: {e}. Run `sudo tunerctl fan auto {}` or reboot the Mac.",
                    self.index, self.index
                ),
            }
        }
    }
}

fn cpu_temp<T: Transport>(smc: &mut Smc<T>) -> Option<(FourCC, f64)> {
    for name in ["TCXC", "TC0P", "TC0D", "TC0E"] {
        let key: FourCC = name.parse().ok()?;
        if let Ok(c) = smc.read_f64(key) {
            if c > 0.0 && c < 130.0 {
                return Some((key, c));
            }
        }
    }
    None
}

pub fn require_root() -> Result<(), SmcError> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { geteuid() } != 0 {
        return Err(SmcError::InvalidValue(
            "this command writes to the SMC and needs root: run with sudo".into(),
        ));
    }
    Ok(())
}

pub fn run<T: Transport>(smc: &mut Smc<T>, opts: &Options) -> Result<(), SmcError> {
    require_root()?;

    let report = smc.fan_report()?;
    let fan = report
        .fans
        .iter()
        .find(|f| f.index == opts.index)
        .ok_or_else(|| {
            SmcError::InvalidValue(format!(
                "fan {} does not exist (FNum = {})",
                opts.index,
                report.fans.len()
            ))
        })?
        .clone();
    let (Some(min), Some(max)) = (fan.min_rpm, fan.max_rpm) else {
        return Err(SmcError::InvalidValue(format!(
            "fan {} has no F{0}Mn/F{0}Mx",
            opts.index
        )));
    };
    if !(min..=max).contains(&opts.rpm) {
        return Err(SmcError::InvalidValue(format!(
            "{:.0} RPM out of fan {}'s range ({min:.0} to {max:.0})",
            opts.rpm, opts.index
        )));
    }
    if report.mode_key == smc::ModeKey::PerFan {
        eprintln!("warning: Mac with F{{n}}Md (T2) — this path is not yet tested on real hardware");
    }

    // SAFETY: the handler only writes to an AtomicBool, which is safe inside a signal.
    unsafe {
        signal(SIGINT, on_signal);
        signal(SIGTERM, on_signal);
        signal(SIGHUP, on_signal);
    }

    println!(
        "Fan {} ({}): {:.0} RPM now, range {min:.0}–{max:.0}, mode {:?}",
        fan.index,
        fan.name.as_deref().unwrap_or("unnamed"),
        fan.actual_rpm.unwrap_or(0.0),
        fan.mode
    );
    println!(
        "Test: {:.0} RPM for {} s. Ctrl+C returns control before the end.\n",
        opts.rpm, opts.seconds
    );

    let mut guard = RestoreGuard {
        smc,
        index: opts.index,
        mode_key: report.mode_key,
        original: Original {
            mode: fan.mode,
            target: fan.target_rpm,
        },
        done: false,
    };

    guard
        .smc
        .set_fan_manual(opts.index, report.mode_key, true)?;
    guard.smc.set_fan_target(opts.index, opts.rpm)?;

    let actual_key = FourCC::fan(opts.index, keys::SUFFIX_ACTUAL)?;
    let start = Instant::now();
    let mut aborted = None;
    for second in 1..=opts.seconds {
        while start.elapsed() < Duration::from_secs(second) {
            if STOP.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if STOP.load(Ordering::SeqCst) {
            aborted = Some("interrupted by the user".to_string());
            break;
        }
        let actual = guard.smc.read_f64(actual_key).unwrap_or(f64::NAN);
        let mode = guard
            .smc
            .fan_mode(opts.index, report.mode_key)
            .unwrap_or(FanMode::Unknown);
        let temp = cpu_temp(guard.smc);
        let temp_txt = temp
            .map(|(k, c)| format!("{k} {c:.1} °C"))
            .unwrap_or_else(|| "CPU ?".into());
        println!(
            "t={second:>3}s  actual {actual:>5.0} RPM  target {:.0}  mode {mode:?}  {temp_txt}",
            opts.rpm
        );
        if let Some((_, c)) = temp {
            if c >= PANIC_TEMP_C {
                aborted = Some(format!("CPU at {c:.1} °C (limit {PANIC_TEMP_C} °C)"));
                break;
            }
        }
    }

    guard.restore()?;
    let after = guard
        .smc
        .fan_mode(opts.index, report.mode_key)
        .unwrap_or(FanMode::Unknown);
    match aborted {
        Some(why) => println!("\nAborted: {why}. Fan returned to its original state ({after:?})."),
        None => println!("\nDone. Fan returned to its original state ({after:?})."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Options;

    #[test]
    fn parses_options() {
        let o = Options::parse("0", &["--rpm", "3000", "--seconds", "20"]).unwrap();
        assert_eq!((o.index, o.rpm, o.seconds), (0, 3000.0, 20));
        assert_eq!(Options::parse("1", &["--rpm", "2500"]).unwrap().seconds, 15);
    }

    #[test]
    fn rejects_bad_options() {
        assert!(Options::parse("x", &["--rpm", "3000"]).is_err());
        assert!(Options::parse("0", &[]).is_err());
        assert!(Options::parse("0", &["--rpm"]).is_err());
        assert!(Options::parse("0", &["--rpm", "3000", "--seconds", "0"]).is_err());
        assert!(Options::parse("0", &["--rpm", "3000", "--seconds", "999"]).is_err());
        assert!(Options::parse("0", &["--foo", "1"]).is_err());
    }
}
