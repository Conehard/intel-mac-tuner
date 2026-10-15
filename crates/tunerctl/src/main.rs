//! tunerctl — Intel Mac Tuner. Reads are free; writes and the service need sudo.

mod fan_test;
mod service;
mod system;

use std::process::ExitCode;

use smc::{FanMode, FourCC, ModeKey, Smc, SmcError, Transport};

const USAGE: &str = "\
tunerctl — Intel Mac Tuner

USAGE:
    tunerctl fans              Fans: current, min, max, target RPM and mode
    tunerctl temps             Temperature sensors with a valid reading
    tunerctl smc list          All SMC keys with type, size and value
    tunerctl smc read <KEY>    One key (e.g. F0Ac, TC0P, \"FS! \")
    tunerctl report            fans + temps + control keys (for diagnostics)

WRITE (needs sudo):
    tunerctl fan test <N> --rpm <RPM> [--seconds <S>]
                               Fan N in manual for S seconds (default 15, max 120),
                               then back to the original state. Ctrl+C also restores.
    tunerctl fan auto <N>      Returns fan N to the SMC's automatic control
    (fan test/auto refuse to run with the daemon active, to avoid fighting over the fan)

SYSTEM (safe, reversible optimization):
    tunerctl system report        Report: what makes this Mac slow (read-only)
    tunerctl system trim visual   Applies the light visual effects (user)
    tunerctl system trim updaters Disables third-party updaters (user; OCLP/yours stay)
    tunerctl system trim apple    Disables optional Apple services (daemon; photo/media analysis, Siri)
    tunerctl system spotlight off Pauses Spotlight indexing (daemon)
    tunerctl system spotlight on  Resumes Spotlight indexing (daemon)
    tunerctl system purge         Frees inactive memory now (daemon)
    tunerctl system restore       Undoes everything system trim changed
    (actions marked \"daemon\" need the daemon running: sudo tunerctl install)

MODE (with the daemon running; no sudo for administrators):
    tunerctl mode              Active mode, temperature and RPM
    tunerctl mode auto         SMC in control
    tunerctl mode fixed <RPM>  Fixed RPM
    tunerctl mode sensor <MIN> <MAX>
                               Curve: MIN °C -> min RPM, MAX °C -> max RPM
    (the change is saved in config.toml and persists after a reboot)

TURBO BOOST (needs the kext; with the daemon running):
    sudo tunerctl install-turbo [kext]   Installs the DisableTurbo kext (root:wheel)
    tunerctl turbo             Turbo Boost state
    tunerctl turbo off         Turns Turbo Boost off (the daemon loads the kext)
    tunerctl turbo on          Turns Turbo Boost on (the daemon unloads the kext)

DAEMON:
    sudo tunerctl install      Installs into /usr/local/bin and starts the LaunchDaemon (runs at boot)
    sudo tunerctl reload       Validates config.toml and tells the daemon to reload
    tunerctl status            Daemon state, configured mode and the last log lines
    tunerctl config check [FILE] Validates a config.toml without applying it
    sudo tunerctl uninstall [--purge]
                               Stops and removes the daemon (--purge deletes config and log)

    tunerctl help              This help
";

enum Command<'a> {
    Fans,
    Temps,
    List,
    Read(&'a str),
    Report,
    FanTest(fan_test::Options),
    FanAuto(u8),
}

/// Commands that don't need to open the SMC.
fn service_command(args: &[&str]) -> Option<Result<(), String>> {
    Some(match args {
        ["install"] => service::install(),
        ["uninstall"] => service::uninstall(false),
        ["uninstall", "--purge"] => service::uninstall(true),
        ["reload"] => service::reload(),
        ["status"] => service::status(),
        ["mode", rest @ ..] => service::mode(rest),
        ["turbo", rest @ ..] => service::turbo(rest),
        ["install-turbo"] => service::install_turbo(None),
        ["install-turbo", path] => service::install_turbo(Some(path)),
        ["config", "check"] => service::config_check(None).map(|_| ()),
        ["config", "check", path] => service::config_check(Some(path)).map(|_| ()),
        ["system", rest @ ..] => system::command(rest),
        _ => return None,
    })
}

fn refuse_if_daemon_running() -> Result<(), SmcError> {
    if let Some(pid) = service::daemon_state() {
        let pid = pid.map(|p| format!(" (pid {p})")).unwrap_or_default();
        return Err(SmcError::InvalidValue(format!(
            "the daemon is active{pid} and controls the fans. Use config.toml + `sudo tunerctl reload`, \
             or stop the daemon with `sudo launchctl bootout system/{}`",
            tuner::paths::LABEL
        )));
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    if matches!(args.first().copied(), None | Some("help" | "-h" | "--help")) {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    if let Some(result) = service_command(&args) {
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    let command = match args.as_slice() {
        ["fans"] => Command::Fans,
        ["temps"] => Command::Temps,
        ["smc", "list"] => Command::List,
        ["smc", "read", key] => Command::Read(key),
        ["report"] => Command::Report,
        ["fan", "test", index, rest @ ..] => match fan_test::Options::parse(index, rest) {
            Ok(o) => Command::FanTest(o),
            Err(e) => {
                eprintln!("error: {e}\n\n{USAGE}");
                return ExitCode::from(2);
            }
        },
        ["fan", "auto", index] => match index.parse() {
            Ok(i) => Command::FanAuto(i),
            Err(_) => {
                eprintln!("error: invalid fan: {index:?}");
                return ExitCode::from(2);
            }
        },
        _ => {
            eprintln!("unknown command: {}\n\n{USAGE}", args.join(" "));
            return ExitCode::from(2);
        }
    };

    let result = smc::open().and_then(|mut smc| match command {
        Command::Fans => cmd_fans(&mut smc),
        Command::Temps => cmd_temps(&mut smc),
        Command::List => cmd_list(&mut smc),
        Command::Read(key) => cmd_read(&mut smc, key),
        Command::Report => cmd_report(&mut smc),
        Command::FanTest(o) => {
            refuse_if_daemon_running().and_then(|()| fan_test::run(&mut smc, &o))
        }
        Command::FanAuto(i) => refuse_if_daemon_running().and_then(|()| cmd_fan_auto(&mut smc, i)),
    });

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn rpm(v: Option<f64>) -> String {
    v.map(|r| format!("{r:.0}")).unwrap_or_else(|| "-".into())
}

fn cmd_fans<T: Transport>(smc: &mut Smc<T>) -> Result<(), SmcError> {
    let r = smc.fan_report()?;
    let mode_key = match r.mode_key {
        ModeKey::PerFan => "F{n}Md (per-fan, T2 style)",
        ModeKey::Bitmask => "FS! (bitmask, pre-T2)",
        ModeKey::Unknown => "none found",
    };
    println!("Manual mode key: {mode_key}");
    println!("Ftst present: {}", if r.has_ftst { "yes" } else { "no" });
    println!();
    println!(
        "{:<4} {:<14} {:>7} {:>7} {:>7} {:>7} {:>7}  {:<5} {:<6}",
        "FAN", "NAME", "ACTUAL", "MIN", "MAX", "TARGET", "SAFE", "TYPE", "MODE"
    );
    for f in &r.fans {
        let mode = match f.mode {
            FanMode::Auto => "auto",
            FanMode::Manual => "manual",
            FanMode::Unknown => "?",
        };
        println!(
            "{:<4} {:<14} {:>7} {:>7} {:>7} {:>7} {:>7}  {:<5} {:<6}",
            f.index,
            f.name.as_deref().unwrap_or("-"),
            rpm(f.actual_rpm),
            rpm(f.min_rpm),
            rpm(f.max_rpm),
            rpm(f.target_rpm),
            rpm(f.safe_rpm),
            f.rpm_type
                .map(|t| t.to_string())
                .unwrap_or_else(|| "-".into()),
            mode
        );
    }
    if r.fans.is_empty() {
        println!("(no fan reported by FNum)");
    }
    Ok(())
}

fn cmd_temps<T: Transport>(smc: &mut Smc<T>) -> Result<(), SmcError> {
    let temps = smc.temperatures()?;
    println!("{:<6} {:>8}  {:<5} SENSOR", "KEY", "°C", "TYPE");
    for s in &temps {
        println!(
            "{:<6} {:>8.1}  {:<5} {}",
            s.key.to_string(),
            s.celsius,
            s.data_type.to_string(),
            s.label.unwrap_or("")
        );
    }
    println!(
        "\n{} sensor(s) with a reading between 0 and 130 °C",
        temps.len()
    );
    Ok(())
}

fn cmd_list<T: Transport>(smc: &mut Smc<T>) -> Result<(), SmcError> {
    let keys = smc.all_keys()?;
    println!("{:<6} {:<5} {:>4}  VALUE", "KEY", "TYPE", "SIZE");
    for key in &keys {
        match smc.read_raw(*key) {
            Ok((info, bytes)) => println!(
                "{:<6} {:<5} {:>4}  {}",
                key.to_string(),
                info.data_type.to_string(),
                info.size,
                smc::value::decode(info.data_type, &bytes)
            ),
            Err(e) => println!("{:<6} {:<5} {:>4}  ({e})", key.to_string(), "?", "?"),
        }
    }
    println!("\n{} key(s)", keys.len());
    Ok(())
}

fn cmd_read<T: Transport>(smc: &mut Smc<T>, key: &str) -> Result<(), SmcError> {
    let key: FourCC = key.parse()?;
    let (info, bytes) = smc.read_raw(key)?;
    println!("key:        {key:?}");
    println!("type:       {}", info.data_type);
    println!("size:       {}", info.size);
    println!("attributes: 0x{:02x}", info.attributes);
    print!("bytes:      ");
    for b in &bytes {
        print!("{b:02x} ");
    }
    println!();
    println!("value:      {}", smc::value::decode(info.data_type, &bytes));
    Ok(())
}

fn cmd_report<T: Transport>(smc: &mut Smc<T>) -> Result<(), SmcError> {
    println!("== Fans");
    cmd_fans(smc)?;
    println!("\n== Control keys");
    for name in [
        "FNum", "FS! ", "Ftst", "F0Md", "F1Md", "F0md", "F0Tg", "F0Mn",
    ] {
        let key: FourCC = name.parse()?;
        match smc.read_raw(key) {
            Ok((info, bytes)) => println!(
                "{name:<5} {:<5} {:>2}  {}",
                info.data_type.to_string(),
                info.size,
                smc::value::decode(info.data_type, &bytes)
            ),
            Err(SmcError::KeyNotFound(_)) => println!("{name:<5} (does not exist)"),
            Err(e) => println!("{name:<5} ({e})"),
        }
    }
    println!("\n== Temperatures");
    cmd_temps(smc)
}

fn cmd_fan_auto<T: Transport>(smc: &mut Smc<T>, index: u8) -> Result<(), SmcError> {
    fan_test::require_root()?;
    let mode_key = smc.detect_mode_key()?;
    if index >= smc.fan_count()? {
        return Err(SmcError::InvalidValue(format!(
            "fan {index} does not exist"
        )));
    }
    smc.set_fan_manual(index, mode_key, false)?;
    println!("fan {index}: {:?}", smc.fan_mode(index, mode_key)?);
    Ok(())
}
