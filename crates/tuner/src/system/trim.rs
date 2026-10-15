//! Applying and undoing the user-level optimizations.

use super::*;

// ---------- trim / restore ----------

#[derive(Debug, Clone, PartialEq)]
pub struct VisualOutcome {
    pub changed: usize,
    pub needs_relogin: bool,
}

/// Applies the light visual effects. Idempotent; records the previous state.
pub fn trim_visual() -> Result<VisualOutcome, String> {
    let mut journal = read_journal();
    let mut changed = 0;
    let mut need_dock = false;
    let mut relogin = false;
    for t in VISUAL_TARGETS {
        let raw = defaults_read(t.domain, t.key);
        let already_light = matches!(raw.as_deref(), Some("1") if t.light_value)
            || matches!(raw.as_deref(), Some("0") if !t.light_value);
        if already_light {
            continue;
        }
        let present = raw.is_some();
        if !journal
            .iter()
            .any(|e| matches!(e, JournalEntry::Default { domain, key, .. } if domain == t.domain && key == t.key))
        {
            journal.push(JournalEntry::Default {
                domain: t.domain.to_string(),
                key: t.key.to_string(),
                present,
                old: raw.unwrap_or_default(),
            });
        }
        if !defaults_write_bool(t.domain, t.key, t.light_value) {
            return Err(format!("failed to apply {}", t.label));
        }
        changed += 1;
        need_dock |= t.dock;
        if t.key == "reduceTransparency" {
            relogin = true;
        }
    }
    write_journal(&journal)?;
    if need_dock {
        let _ = run_ok("/usr/bin/killall", &["Dock"]);
    }
    Ok(VisualOutcome {
        changed,
        needs_relogin: relogin,
    })
}

/// Disables the user-level third-party updaters. Returns the disabled labels.
pub fn trim_updaters() -> Result<Vec<String>, String> {
    let uid = uid().ok_or("could not get the user's uid")?;
    let domain = format!("gui/{uid}");
    let mut journal = read_journal();
    let mut done = Vec::new();
    for a in collect().disableable_updaters() {
        let target = format!("{domain}/{}", a.label);
        // Stop now and disable persistently.
        let _ = run_ok("/bin/launchctl", &["bootout", &target]);
        if !run_ok("/bin/launchctl", &["disable", &target]) {
            // some may already be disabled; keep going
        }
        let plist = a
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !journal
            .iter()
            .any(|e| matches!(e, JournalEntry::Agent { label, .. } if *label == a.label))
        {
            journal.push(JournalEntry::Agent {
                label: a.label.clone(),
                plist,
            });
        }
        done.push(a.label.clone());
    }
    write_journal(&journal)?;
    Ok(done)
}

/// Undoes a single journal entry (without touching the file).
/// Returns Ok(Dock needs restart) or Err if the daemon operation failed.
pub(super) fn undo_entry(e: &JournalEntry, uid: &Option<String>) -> Result<bool, String> {
    match e {
        JournalEntry::Default {
            domain,
            key,
            present,
            old,
        } => {
            if *present {
                defaults_write_bool(domain, key, old == "1" || old.eq_ignore_ascii_case("true"));
            } else {
                defaults_delete(domain, key);
            }
            Ok(domain == "com.apple.dock")
        }
        JournalEntry::Agent { label, plist } => {
            if let Some(uid) = uid {
                let target = format!("gui/{uid}/{label}");
                let _ = run_ok("/bin/launchctl", &["enable", &target]);
                if !plist.is_empty() {
                    let _ = run_ok(
                        "/bin/launchctl",
                        &["bootstrap", &format!("gui/{uid}"), plist],
                    );
                }
            }
            Ok(false)
        }
        JournalEntry::Spotlight => {
            daemon(&crate::ipc::Request::SetSpotlight { indexing: true })?;
            Ok(false)
        }
        JournalEntry::AppleAgent { label, uid } => {
            daemon(&crate::ipc::Request::SetAppleAgent {
                enable: true,
                uid: *uid,
                label: label.clone(),
            })?;
            Ok(false)
        }
    }
}

/// Undoes the entries that should NOT stay in the journal and rewrites the rest.
/// Entries whose reversal failed (e.g. daemon down) stay in the journal for a
/// retry, and the function returns Err with the reason(s).
pub(super) fn restore_where(keep_filter: impl Fn(&JournalEntry) -> bool) -> Result<usize, String> {
    let entries = read_journal();
    if entries.is_empty() {
        return Ok(0);
    }
    let uid = uid();
    let mut need_dock = false;
    let mut undone = 0;
    let mut remaining = Vec::new();
    let mut errors = Vec::new();
    for e in entries {
        if keep_filter(&e) {
            remaining.push(e);
            continue;
        }
        match undo_entry(&e, &uid) {
            Ok(dock) => {
                need_dock |= dock;
                undone += 1;
            }
            Err(msg) => {
                errors.push(msg);
                remaining.push(e);
            }
        }
    }
    if need_dock {
        let _ = run_ok("/usr/bin/killall", &["Dock"]);
    }
    write_journal(&remaining)?;
    if errors.is_empty() {
        Ok(undone)
    } else {
        Err(errors.join("; "))
    }
}

/// Undoes everything `trim` changed, from the journal.
pub fn restore() -> Result<usize, String> {
    restore_where(|_| false)
}

/// Undoes only the visual effects (keeps the rest of the journal).
pub fn restore_visual() -> Result<usize, String> {
    restore_where(|e| !matches!(e, JournalEntry::Default { .. }))
}

/// Re-enables only the third-party updaters (keeps the rest).
pub fn restore_updaters() -> Result<usize, String> {
    restore_where(|e| !matches!(e, JournalEntry::Agent { .. }))
}

/// Re-enables only the Apple services (keeps the rest).
pub fn restore_apple_services() -> Result<usize, String> {
    restore_where(|e| !matches!(e, JournalEntry::AppleAgent { .. }))
}

/// Resumes Spotlight (keeps the rest).
pub fn resume_spotlight() -> Result<usize, String> {
    restore_where(|e| !matches!(e, JournalEntry::Spotlight))
}
