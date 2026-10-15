//! Thin wrappers around the system commands used for collection and actions.

use std::path::PathBuf;
use std::process::Command;

// ---------- base IO ----------

pub(super) fn sh(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

pub(super) fn run_ok(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub(super) fn sysctl(key: &str) -> Option<String> {
    sh("/usr/sbin/sysctl", &["-n", key]).filter(|s| !s.is_empty())
}

pub(super) fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub(super) fn uid() -> Option<String> {
    sh("/usr/bin/id", &["-u"])
}

pub(super) fn defaults_read(domain: &str, key: &str) -> Option<String> {
    sh("/usr/bin/defaults", &["read", domain, key])
}

pub(super) fn defaults_write_bool(domain: &str, key: &str, value: bool) -> bool {
    run_ok(
        "/usr/bin/defaults",
        &[
            "write",
            domain,
            key,
            "-bool",
            if value { "true" } else { "false" },
        ],
    )
}

pub(super) fn defaults_delete(domain: &str, key: &str) -> bool {
    run_ok("/usr/bin/defaults", &["delete", domain, key])
}
