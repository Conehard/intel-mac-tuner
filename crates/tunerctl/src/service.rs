//! LaunchDaemon installation and management: install, uninstall, reload, status, config check.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use tuner::{paths, Config, Mode};

use crate::fan_test::require_root;

const LAUNCHCTL: &str = "/bin/launchctl";

fn target() -> String {
    format!("system/{}", paths::LABEL)
}

fn launchctl(args: &[&str]) -> Result<String, String> {
    let out = Command::new(LAUNCHCTL)
        .args(args)
        .output()
        .map_err(|e| format!("could not run launchctl: {e}"))?;
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!(
            "launchctl {} failed: {}",
            args.join(" "),
            text.trim()
        ))
    }
}

/// Is the LaunchDaemon loaded? Returns the PID if it is running.
pub fn daemon_state() -> Option<Option<u32>> {
    let out = launchctl(&["print", &target()]).ok()?;
    let pid = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("pid = "))
        .and_then(|p| p.trim().parse().ok());
    Some(pid)
}

fn wait_unloaded() -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if daemon_state().is_none() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn stop_daemon() -> Result<(), String> {
    if daemon_state().is_some() {
        launchctl(&["bootout", &target()])?;
        if !wait_unloaded() {
            return Err("the daemon did not stop within 10 s".into());
        }
        println!("daemon stopped (fans returned to the SMC)");
    }
    Ok(())
}

/// Copies to `dst.tmp` and renames: never leaves a half-written binary.
fn install_file(src: &Path, dst: &Path, mode: u32) -> Result<(), String> {
    if src == dst {
        return Ok(());
    }
    let tmp = dst.with_extension("tmp");
    fs::copy(src, &tmp).map_err(|e| format!("copy {} -> {}: {e}", src.display(), tmp.display()))?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
    fs::rename(&tmp, dst).map_err(|e| format!("rename {}: {e}", dst.display()))?;
    println!("installed {}", dst.display());
    Ok(())
}

fn write_file(path: &Path, content: &str, mode: u32) -> Result<(), String> {
    let mut f = fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    f.write_all(content.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("chmod {}: {e}", path.display()))
}

pub fn describe(cfg: &Config) -> String {
    match cfg.mode {
        Mode::Auto => "auto (SMC in control)".into(),
        Mode::Fixed => format!("fixed, {:.0} RPM", cfg.fixed_rpm),
        Mode::Sensor => {
            let s = &cfg.sensor;
            let range = match (s.rpm_min, s.rpm_max) {
                (None, None) => "fan range".to_string(),
                (lo, hi) => format!(
                    "{}–{} RPM",
                    lo.map_or("min".into(), |v| format!("{v:.0}")),
                    hi.map_or("max".into(), |v| format!("{v:.0}"))
                ),
            };
            format!(
                "sensor, {} {:.0}–{:.0} °C -> {range}, hysteresis {:.1} °C",
                s.key, s.temp_min_c, s.temp_max_c, s.hysteresis_c
            )
        }
    }
}

pub fn install() -> Result<(), String> {
    require_root().map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let dir = exe.parent().ok_or("unknown tunerctl directory")?;
    let daemon_src = dir.join("intelmactunerd");
    if !daemon_src.exists() {
        return Err(format!(
            "{} not found. Run `cargo build --release` and use ./target/release/tunerctl install",
            daemon_src.display()
        ));
    }

    stop_daemon()?;

    fs::create_dir_all(paths::BIN_DIR).map_err(|e| format!("{}: {e}", paths::BIN_DIR))?;
    install_file(&daemon_src, Path::new(paths::DAEMON_BIN), 0o755)?;
    install_file(&exe, Path::new(paths::CTL_BIN), 0o755)?;

    fs::create_dir_all(paths::CONFIG_DIR).map_err(|e| format!("{}: {e}", paths::CONFIG_DIR))?;
    let config = Path::new(paths::CONFIG);
    if config.exists() {
        match Config::load(config) {
            Ok(c) => println!("existing configuration kept: {}", describe(&c)),
            Err(e) => {
                println!("WARNING: existing configuration has an error ({e}); the daemon will use auto mode")
            }
        }
    } else {
        write_file(config, tuner::DEFAULT_CONFIG, 0o644)?;
        println!(
            "default configuration created at {} (auto mode)",
            config.display()
        );
    }

    write_file(Path::new(paths::PLIST), &paths::launchd_plist(), 0o644)?;
    println!("installed {}", paths::PLIST);
    launchctl(&["bootstrap", "system", paths::PLIST])?;
    std::thread::sleep(Duration::from_millis(800));

    match daemon_state() {
        Some(Some(pid)) => {
            println!("\ndaemon running (pid {pid}). It starts on its own at every boot.")
        }
        Some(None) => println!("\ndaemon loaded, but no PID yet. See: tail {}", paths::LOG),
        None => return Err(format!("the daemon did not load. See: tail {}", paths::LOG)),
    }
    println!("Edit {} and run `sudo tunerctl reload`.", paths::CONFIG);
    Ok(())
}

pub fn uninstall(purge: bool) -> Result<(), String> {
    require_root().map_err(|e| e.to_string())?;
    stop_daemon()?;

    // Re-enable Turbo and unload the kext, if loaded.
    let turbo = tuner::turbo::Turbo::new(tuner::turbo::SystemKext);
    if let Err(e) = turbo.set_enabled(true) {
        // Not fatal: it may not even be installed.
        if !e.contains("not installed") {
            println!("warning re-enabling Turbo: {e}");
        }
    }

    for p in [paths::PLIST, paths::DAEMON_BIN, paths::CTL_BIN] {
        if Path::new(p).exists() {
            fs::remove_file(p).map_err(|e| format!("remove {p}: {e}"))?;
            println!("removed {p}");
        }
    }
    if Path::new(paths::TURBO_KEXT).exists() {
        fs::remove_dir_all(paths::TURBO_KEXT)
            .map_err(|e| format!("remove {}: {e}", paths::TURBO_KEXT))?;
        println!("removed {}", paths::TURBO_KEXT);
    }
    if purge {
        if Path::new(paths::CONFIG_DIR).exists() {
            fs::remove_dir_all(paths::CONFIG_DIR)
                .map_err(|e| format!("remove {}: {e}", paths::CONFIG_DIR))?;
            println!("removed {}", paths::CONFIG_DIR);
        }
        if Path::new(paths::LOG).exists() {
            fs::remove_file(paths::LOG).map_err(|e| format!("remove {}: {e}", paths::LOG))?;
            println!("removed {}", paths::LOG);
        }
    } else {
        println!(
            "configuration kept at {} (use --purge to delete)",
            paths::CONFIG
        );
    }

    // Extra safety: fans back to Auto even if the daemon died without restoring.
    if let Ok(mut smc) = smc::open() {
        if let (Ok(mode_key), Ok(n)) = (smc.detect_mode_key(), smc.fan_count()) {
            for i in 0..n {
                let _ = smc.set_fan_manual(i, mode_key, false);
            }
            println!("fans returned to the SMC");
        }
    }
    Ok(())
}

pub fn config_check(path: Option<&str>) -> Result<Config, String> {
    let path = PathBuf::from(path.unwrap_or(paths::CONFIG));
    let cfg = Config::load(&path)?;
    println!("{}: ok — {}", path.display(), describe(&cfg));
    Ok(cfg)
}

pub fn reload() -> Result<(), String> {
    require_root().map_err(|e| e.to_string())?;
    config_check(None)?;
    if daemon_state().is_none() {
        return Err("the daemon is not installed. Run `sudo tunerctl install`".into());
    }
    launchctl(&["kill", "1", &target()])?; // 1 = SIGHUP
    println!("daemon notified. See the result: tail -n 5 {}", paths::LOG);
    Ok(())
}

pub fn status() -> Result<(), String> {
    match daemon_state() {
        Some(Some(pid)) => println!("daemon: running (pid {pid})"),
        Some(None) => println!("daemon: loaded, stopped (launchd will try to restart)"),
        None if Path::new(paths::PLIST).exists() => {
            println!("daemon: plist installed, but launchctl did not show the service (try `sudo tunerctl status`)")
        }
        None => println!("daemon: not installed"),
    }
    match tuner::ipc::status() {
        Ok(st) => print_live(&st),
        Err(e) => println!("socket: {e}"),
    }
    match Config::load(Path::new(paths::CONFIG)) {
        Ok(c) => println!("config: {}", describe(&c)),
        Err(e) if Path::new(paths::CONFIG).exists() => println!("config: ERROR — {e}"),
        Err(_) => println!("config: {} does not exist", paths::CONFIG),
    }
    if let Ok(text) = fs::read_to_string(paths::LOG) {
        let lines: Vec<&str> = text.lines().collect();
        println!("\nlog ({}), last lines:", paths::LOG);
        for l in &lines[lines.len().saturating_sub(8)..] {
            println!("  {l}");
        }
    }
    Ok(())
}

fn print_live(st: &tuner::ipc::Status) {
    let mode = match st.mode.as_str() {
        "fixed" => format!("fixed, {:.0} RPM", st.fixed_rpm),
        "sensor" => format!(
            "sensor, {} {:.0}–{:.0} °C",
            st.sensor_key, st.temp_min_c, st.temp_max_c
        ),
        other => other.to_string(),
    };
    println!(
        "active mode: {mode}{}",
        if st.panic { "  [THERMAL PANIC]" } else { "" }
    );
    match &st.temp {
        Some((k, c)) => println!("temperature: {k} {c:.1} °C"),
        None => println!("temperature: no reading"),
    }
    for f in &st.fans {
        println!(
            "fan {}: {:.0} RPM (range {:.0}–{:.0})",
            f.index, f.rpm, f.min, f.max
        );
    }
    if st.turbo != tuner::ipc::TurboStatus::Unmanaged {
        print_turbo(st);
    }
}

/// `tunerctl mode [auto | fixed <rpm> | sensor <min> <max>]` — via socket, no sudo for admins.
pub fn mode(args: &[&str]) -> Result<(), String> {
    use tuner::ipc::{self, Request};
    let num = |s: &str| {
        s.parse::<f64>()
            .map_err(|_| format!("invalid number: {s:?}"))
    };
    let req = match args {
        [] => {
            print_live(&ipc::status()?);
            return Ok(());
        }
        ["auto"] => Request::SetAuto,
        ["fixed", rpm] => Request::SetFixed { rpm: num(rpm)? },
        ["sensor", lo, hi] => Request::SetSensor {
            temp_min_c: num(lo)?,
            temp_max_c: num(hi)?,
        },
        _ => {
            return Err(
                "usage: tunerctl mode [auto | fixed <rpm> | sensor <temp_min> <temp_max>]".into(),
            )
        }
    };
    ipc::set(&req)?;
    std::thread::sleep(Duration::from_millis(1500));
    print_live(&ipc::status()?);
    Ok(())
}

// ---------- Turbo Boost ----------

fn print_turbo(st: &tuner::ipc::Status) {
    use tuner::ipc::TurboStatus::*;
    let txt = match st.turbo {
        Enabled => "on",
        Disabled => "off",
        Unmanaged => "not managed by the daemon (turbo.manage = false)",
        Unknown => "managed, but no reading (kext installed?)",
    };
    println!("Turbo Boost: {txt}");
}

/// `tunerctl turbo [on | off | status]` — via socket.
pub fn turbo(args: &[&str]) -> Result<(), String> {
    use tuner::ipc::{self, Request};
    let set_and_show = |enabled: bool| -> Result<(), String> {
        ipc::set(&Request::SetTurbo { enabled })?;
        std::thread::sleep(Duration::from_millis(800));
        print_turbo(&ipc::status()?);
        Ok(())
    };
    match args {
        [] | ["status"] => {
            print_turbo(&ipc::status()?);
            Ok(())
        }
        ["on"] => set_and_show(true),
        ["off"] => set_and_show(false),
        _ => Err("usage: tunerctl turbo [on | off | status]".into()),
    }
}

/// Looks for the built DisableTurbo.kext (from build-kext.sh).
fn find_kext_source(explicit: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        let p = PathBuf::from(p);
        return if p.join("Contents/MacOS/DisableTurbo").exists() {
            Ok(p)
        } else {
            Err(format!(
                "{} does not look like a valid DisableTurbo.kext",
                p.display()
            ))
        };
    }
    let mut candidates = vec![
        PathBuf::from("kext/DisableTurbo.kext"),
        PathBuf::from("DisableTurbo.kext"),
    ];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // target/release/tunerctl -> ../../kext/DisableTurbo.kext
            candidates.push(dir.join("../../kext/DisableTurbo.kext"));
        }
    }
    candidates
        .into_iter()
        .find(|p| p.join("Contents/MacOS/DisableTurbo").exists())
        .ok_or_else(|| {
            "DisableTurbo.kext not found. Build it first: sh kext/build-kext.sh\n\
             or pass the path: sudo tunerctl install-turbo <DisableTurbo.kext>"
                .to_string()
        })
}

/// `sudo tunerctl install-turbo [kext]` — installs the kext into TURBO_KEXT (root:wheel).
pub fn install_turbo(path: Option<&str>) -> Result<(), String> {
    require_root().map_err(|e| e.to_string())?;
    let src = find_kext_source(path)?;
    let dst = Path::new(paths::TURBO_KEXT);

    fs::create_dir_all(paths::CONFIG_DIR).map_err(|e| format!("{}: {e}", paths::CONFIG_DIR))?;
    if dst.exists() {
        fs::remove_dir_all(dst).map_err(|e| format!("remove old {}: {e}", dst.display()))?;
    }
    // cp -R preserves the bundle structure.
    let cp = Command::new("/bin/cp")
        .args(["-R", &src.to_string_lossy(), &dst.to_string_lossy()])
        .status()
        .map_err(|e| format!("cp: {e}"))?;
    if !cp.success() {
        return Err("failed to copy the kext".into());
    }
    let chown = Command::new("/usr/sbin/chown")
        .args(["-R", "root:wheel", &dst.to_string_lossy()])
        .status()
        .map_err(|e| format!("chown: {e}"))?;
    if !chown.success() {
        return Err("chown root:wheel failed (needs sudo)".into());
    }
    println!("kext installed at {}", dst.display());
    println!();
    println!("To let the daemon control Turbo, turn management on:");
    println!("  tunerctl turbo off    (turns Turbo off now and saves the default)");
    println!("  tunerctl turbo on     (re-enables)");
    println!();
    println!("On the first load, macOS may ask you to approve the extension in");
    println!("System Settings › Privacy & Security, and reboot.");
    Ok(())
}
