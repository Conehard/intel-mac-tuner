//! Operations that need root: client side (ask the daemon) and the runners the daemon calls.

use super::*;

// ---------- operations via the daemon (need root) ----------

/// Sends a request to the daemon; clear error if it is not running.
pub(super) fn daemon(req: &crate::ipc::Request) -> Result<(), String> {
    crate::ipc::set(req)
}

/// Pauses Spotlight indexing (via the daemon) and records it in the journal.
pub fn pause_spotlight() -> Result<(), String> {
    daemon(&crate::ipc::Request::SetSpotlight { indexing: false })?;
    let mut j = read_journal();
    if !j.iter().any(|e| matches!(e, JournalEntry::Spotlight)) {
        j.push(JournalEntry::Spotlight);
        write_journal(&j)?;
    }
    Ok(())
}

/// Frees inactive memory now (via the daemon). One-shot, not journaled.
pub fn purge_memory() -> Result<(), String> {
    daemon(&crate::ipc::Request::PurgeMemory)
}

/// Disables the present optional Apple services (via the daemon). Returns the labels.
pub fn disable_apple_services() -> Result<Vec<String>, String> {
    let uid: u32 = uid()
        .ok_or("could not get the user's uid")?
        .parse()
        .map_err(|_| "invalid uid".to_string())?;
    let mut j = read_journal();
    let mut done = Vec::new();
    for svc in collect_apple_services() {
        if svc.disabled {
            continue;
        }
        daemon(&crate::ipc::Request::SetAppleAgent {
            enable: false,
            uid,
            label: svc.label.to_string(),
        })?;
        if !j
            .iter()
            .any(|e| matches!(e, JournalEntry::AppleAgent { label, .. } if label == svc.label))
        {
            j.push(JournalEntry::AppleAgent {
                label: svc.label.to_string(),
                uid,
            });
        }
        done.push(svc.label.to_string());
    }
    write_journal(&j)?;
    Ok(done)
}

// ---------- privileged runners (called by the daemon, as root) ----------

/// Pauses/resumes Spotlight indexing on all volumes.
pub fn run_spotlight(indexing: bool) -> Result<(), String> {
    let arg = if indexing { "on" } else { "off" };
    if run_ok("/usr/bin/mdutil", &["-a", "-i", arg]) {
        Ok(())
    } else {
        Err(format!("mdutil -a -i {arg} failed"))
    }
}

/// Frees inactive memory.
pub fn run_purge() -> Result<(), String> {
    if run_ok("/usr/sbin/purge", &[]) {
        Ok(())
    } else {
        Err("purge failed".into())
    }
}

/// Enables/disables an Apple agent in the `gui/<uid>` domain. Only accepts the fixed list.
pub fn run_apple_agent(enable: bool, uid: u32, label: &str) -> Result<(), String> {
    if !is_allowed_apple_agent(label) {
        return Err(format!("label not in the allowed list: {label}"));
    }
    let target = format!("gui/{uid}/{label}");
    if enable {
        let _ = run_ok("/bin/launchctl", &["enable", &target]);
        // Comes back on demand; kickstart is just a nudge, it may fail harmlessly.
        let _ = run_ok("/bin/launchctl", &["kickstart", &target]);
    } else {
        let _ = run_ok("/bin/launchctl", &["bootout", &target]);
        // `disable` is what keeps it off after login/reboot.
        let _ = run_ok("/bin/launchctl", &["disable", &target]);
    }
    Ok(())
}
