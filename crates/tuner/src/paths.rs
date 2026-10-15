//! Where each thing is installed.

/// LaunchDaemon label (`launchctl print system/<LABEL>`).
pub const LABEL: &str = "dev.intelmactuner.daemon";
pub const PLIST: &str = "/Library/LaunchDaemons/dev.intelmactuner.daemon.plist";
pub const CONFIG_DIR: &str = "/Library/Application Support/IntelMacTuner";
pub const CONFIG: &str = "/Library/Application Support/IntelMacTuner/config.toml";
/// Installed DisableTurbo kext (root:wheel).
pub const TURBO_KEXT: &str = "/Library/Application Support/IntelMacTuner/DisableTurbo.kext";
pub const LOG: &str = "/var/log/intelmactunerd.log";
/// Control socket (root:admin 0660).
pub const SOCKET: &str = "/var/run/intelmactuner.sock";
pub const BIN_DIR: &str = "/usr/local/bin";
pub const DAEMON_BIN: &str = "/usr/local/bin/intelmactunerd";
pub const CTL_BIN: &str = "/usr/local/bin/tunerctl";

/// Contents of the LaunchDaemon plist.
pub fn launchd_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{DAEMON_BIN}</string>
        <string>--config</string>
        <string>{CONFIG}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>10</integer>
    <key>StandardOutPath</key>
    <string>{LOG}</string>
    <key>StandardErrorPath</key>
    <string>{LOG}</string>
</dict>
</plist>
"#
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn plist_mentions_paths() {
        let p = super::launchd_plist();
        assert!(p.contains("<string>dev.intelmactuner.daemon</string>"));
        assert!(p.contains("/usr/local/bin/intelmactunerd"));
        assert!(p.contains("<key>RunAtLoad</key>\n    <true/>"));
    }
}
