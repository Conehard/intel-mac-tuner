//! Turbo Boost control via the DisableTurbo kext.
//!
//! Convention: kext LOADED = Turbo OFF (loading the kext sets bit 38 of
//! MSR 0x1A0). So "turn Turbo on" = unload the kext, "turn off" = load it.
//!
//! The part that talks to the system (`kextutil`/`kextunload`/`kmutil`) is
//! isolated behind the `KextTool` trait, so the logic is testable without root.

use std::path::Path;
use std::process::Command;

use crate::ipc::TurboStatus;
use crate::paths;

pub const BUNDLE_ID: &str = "dev.intelmactuner.DisableTurbo";

/// Low-level operations on the kext. Real implementation in `SystemKext`.
pub trait KextTool {
    /// Is the kext loaded right now?
    fn is_loaded(&self) -> Result<bool, String>;
    /// Load the kext (starts disabling Turbo). Idempotent.
    fn load(&self) -> Result<(), String>;
    /// Unload the kext (re-enables Turbo). Idempotent.
    fn unload(&self) -> Result<(), String>;
    /// Is the kext installed on disk?
    fn is_installed(&self) -> bool;
}

/// High-level logic: turns "I want Turbo on/off" into load/unload.
pub struct Turbo<K: KextTool> {
    kext: K,
}

impl<K: KextTool> Turbo<K> {
    pub fn new(kext: K) -> Self {
        Turbo { kext }
    }

    /// Current Turbo state (from the "managed" point of view).
    pub fn status(&self) -> TurboStatus {
        if !self.kext.is_installed() {
            return TurboStatus::Unknown;
        }
        match self.kext.is_loaded() {
            Ok(true) => TurboStatus::Disabled,
            Ok(false) => TurboStatus::Enabled,
            Err(_) => TurboStatus::Unknown,
        }
    }

    /// Ensures Turbo is in the desired state. Only acts when needed.
    /// Clear error if the kext is not installed (interactive use).
    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        if !self.kext.is_installed() {
            return Err(format!(
                "DisableTurbo kext not installed at {}. Run: sudo tunerctl install-turbo",
                paths::TURBO_KEXT
            ));
        }
        let loaded = self.kext.is_loaded()?;
        // enabled => kext unloaded; !enabled => loaded.
        match (enabled, loaded) {
            (true, true) => self.kext.unload(),
            (false, false) => self.kext.load(),
            _ => Ok(()), // already in the right state
        }
    }

    /// Reapplies the state at boot and after waking from sleep. Unlike
    /// `set_enabled`: for "off" it forces an MSR rewrite (unload then reload),
    /// because sleep may have cleared the bit even with the kext still loaded.
    /// Silent when the kext is not installed (not an error at boot).
    pub fn reapply(&self, enabled: bool) -> Result<(), String> {
        if !self.kext.is_installed() {
            return Ok(());
        }
        if enabled {
            if self.kext.is_loaded().unwrap_or(false) {
                self.kext.unload()?;
            }
            Ok(())
        } else {
            if self.kext.is_loaded().unwrap_or(false) {
                let _ = self.kext.unload();
            }
            self.kext.load()
        }
    }
}

/// Real implementation: runs the system utilities.
pub struct SystemKext;

fn run(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("could not run {program}: {e}"))
}

/// True if the executable exists and is reachable.
fn have(program: &str) -> bool {
    Path::new(program).exists()
}

impl KextTool for SystemKext {
    fn is_installed(&self) -> bool {
        Path::new(paths::TURBO_KEXT)
            .join("Contents/MacOS/DisableTurbo")
            .exists()
    }

    fn is_loaded(&self) -> Result<bool, String> {
        // kmutil (Big Sur+) is preferred; kextstat is the fallback.
        if have("/usr/bin/kmutil") {
            let out = run(
                "/usr/bin/kmutil",
                &["showloaded", "--bundle-identifier", BUNDLE_ID],
            )?;
            let text = String::from_utf8_lossy(&out.stdout);
            return Ok(text.contains(BUNDLE_ID));
        }
        let out = run("/usr/sbin/kextstat", &["-l", "-b", BUNDLE_ID])?;
        Ok(String::from_utf8_lossy(&out.stdout).contains(BUNDLE_ID))
    }

    fn load(&self) -> Result<(), String> {
        let kext = paths::TURBO_KEXT;
        // The kext must be root:wheel; install-turbo already ensures that.
        let out = if have("/usr/bin/kmutil") {
            run("/usr/bin/kmutil", &["load", "-p", kext])?
        } else {
            run("/sbin/kextload", &[kext])?
        };
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("failed to load the kext: {}", stderr(&out)))
        }
    }

    fn unload(&self) -> Result<(), String> {
        let out = if have("/usr/bin/kmutil") {
            run("/usr/bin/kmutil", &["unload", "-b", BUNDLE_ID])?
        } else {
            run("/sbin/kextunload", &["-b", BUNDLE_ID])?
        };
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("failed to unload the kext: {}", stderr(&out)))
        }
    }
}

fn stderr(out: &std::process::Output) -> String {
    let s = String::from_utf8_lossy(&out.stderr);
    let s = s.trim();
    if s.is_empty() {
        format!("code {}", out.status.code().unwrap_or(-1))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Fake kext: records the actions and lets you control the state.
    struct FakeKext {
        installed: bool,
        loaded: RefCell<bool>,
        log: RefCell<Vec<&'static str>>,
        fail_load: bool,
    }

    impl FakeKext {
        fn new(installed: bool, loaded: bool) -> Self {
            FakeKext {
                installed,
                loaded: RefCell::new(loaded),
                log: RefCell::new(vec![]),
                fail_load: false,
            }
        }
    }

    impl KextTool for FakeKext {
        fn is_installed(&self) -> bool {
            self.installed
        }
        fn is_loaded(&self) -> Result<bool, String> {
            Ok(*self.loaded.borrow())
        }
        fn load(&self) -> Result<(), String> {
            if self.fail_load {
                return Err("boom".into());
            }
            self.log.borrow_mut().push("load");
            *self.loaded.borrow_mut() = true;
            Ok(())
        }
        fn unload(&self) -> Result<(), String> {
            self.log.borrow_mut().push("unload");
            *self.loaded.borrow_mut() = false;
            Ok(())
        }
    }

    #[test]
    fn status_maps_loaded_to_disabled() {
        assert_eq!(
            Turbo::new(FakeKext::new(true, true)).status(),
            TurboStatus::Disabled
        );
        assert_eq!(
            Turbo::new(FakeKext::new(true, false)).status(),
            TurboStatus::Enabled
        );
        assert_eq!(
            Turbo::new(FakeKext::new(false, false)).status(),
            TurboStatus::Unknown
        );
    }

    #[test]
    fn disabling_turbo_loads_kext() {
        let t = Turbo::new(FakeKext::new(true, false));
        t.set_enabled(false).unwrap(); // turn turbo off -> load
        assert_eq!(t.kext.log.borrow().as_slice(), &["load"]);
        assert_eq!(t.status(), TurboStatus::Disabled);
    }

    #[test]
    fn enabling_turbo_unloads_kext() {
        let t = Turbo::new(FakeKext::new(true, true));
        t.set_enabled(true).unwrap(); // turn turbo on -> unload
        assert_eq!(t.kext.log.borrow().as_slice(), &["unload"]);
        assert_eq!(t.status(), TurboStatus::Enabled);
    }

    #[test]
    fn idempotent_when_already_in_state() {
        let t = Turbo::new(FakeKext::new(true, true)); // already off
        t.set_enabled(false).unwrap();
        assert!(t.kext.log.borrow().is_empty(), "should not reload");

        let t = Turbo::new(FakeKext::new(true, false)); // already on
        t.set_enabled(true).unwrap();
        assert!(t.kext.log.borrow().is_empty(), "should not re-unload");
    }

    #[test]
    fn errors_when_not_installed() {
        let t = Turbo::new(FakeKext::new(false, false));
        assert!(t.set_enabled(false).unwrap_err().contains("install-turbo"));
    }

    #[test]
    fn reapply_is_silent_without_kext() {
        let t = Turbo::new(FakeKext::new(false, false));
        assert!(t.reapply(false).is_ok());
    }

    #[test]
    fn reapply_disabled_forces_msr_rewrite() {
        // kext already loaded: reapply(off) should unload and reload (rewrites the MSR).
        let t = Turbo::new(FakeKext::new(true, true));
        t.reapply(false).unwrap();
        assert_eq!(t.kext.log.borrow().as_slice(), &["unload", "load"]);
        assert_eq!(t.status(), TurboStatus::Disabled);
    }

    #[test]
    fn reapply_enabled_just_unloads() {
        let t = Turbo::new(FakeKext::new(true, true));
        t.reapply(true).unwrap();
        assert_eq!(t.kext.log.borrow().as_slice(), &["unload"]);
        assert_eq!(t.status(), TurboStatus::Enabled);
    }

    #[test]
    fn propagates_load_error() {
        let mut k = FakeKext::new(true, false);
        k.fail_load = true;
        assert_eq!(Turbo::new(k).set_enabled(false).unwrap_err(), "boom");
    }
}
