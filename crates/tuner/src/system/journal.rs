//! Reversible journal: entries, serialization and the on-disk file.

use super::*;
use std::path::PathBuf;

// ---------- reversible journal (pure) ----------

#[derive(Debug, Clone, PartialEq)]
pub enum JournalEntry {
    /// A `defaults` change.
    Default {
        domain: String,
        key: String,
        present: bool,
        old: String,
    },
    /// A disabled user LaunchAgent (the plist stays in place).
    Agent { label: String, plist: String },
    /// Spotlight indexing paused by us (undo = resume).
    Spotlight,
    /// An optional Apple service disabled in the `gui/<uid>` domain.
    AppleAgent { label: String, uid: u32 },
}

pub fn serialize_journal(entries: &[JournalEntry]) -> String {
    let mut s = String::new();
    for e in entries {
        match e {
            JournalEntry::Default {
                domain,
                key,
                present,
                old,
            } => s.push_str(&format!(
                "default\t{domain}\t{key}\t{}\t{old}\n",
                u8::from(*present)
            )),
            JournalEntry::Agent { label, plist } => {
                s.push_str(&format!("agent\t{label}\t{plist}\n"));
            }
            JournalEntry::Spotlight => s.push_str("spotlight\n"),
            JournalEntry::AppleAgent { label, uid } => {
                s.push_str(&format!("apple\t{uid}\t{label}\n"));
            }
        }
    }
    s
}

pub fn parse_journal(text: &str) -> Vec<JournalEntry> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(5, '\t').collect();
            match f.as_slice() {
                ["default", domain, key, present, old] => Some(JournalEntry::Default {
                    domain: (*domain).to_string(),
                    key: (*key).to_string(),
                    present: *present == "1",
                    old: (*old).to_string(),
                }),
                ["agent", label, plist] => Some(JournalEntry::Agent {
                    label: (*label).to_string(),
                    plist: (*plist).to_string(),
                }),
                ["spotlight"] => Some(JournalEntry::Spotlight),
                ["apple", uid, label] => Some(JournalEntry::AppleAgent {
                    label: (*label).to_string(),
                    uid: uid.parse().ok()?,
                }),
                _ => None,
            }
        })
        .collect()
}
// ---------- journal on disk ----------

pub(super) fn journal_path() -> PathBuf {
    home().join("Library/Application Support/dev.intelmactuner/system-journal.tsv")
}

pub fn read_journal() -> Vec<JournalEntry> {
    std::fs::read_to_string(journal_path())
        .map(|t| parse_journal(&t))
        .unwrap_or_default()
}

pub(super) fn write_journal(entries: &[JournalEntry]) -> Result<(), String> {
    let path = journal_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if entries.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    std::fs::write(&path, serialize_journal(entries))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Is there any recorded visual-effect change?
pub fn journal_has_visual() -> bool {
    read_journal()
        .iter()
        .any(|e| matches!(e, JournalEntry::Default { .. }))
}

/// Is there any recorded disabled updater?
pub fn journal_has_agents() -> bool {
    read_journal()
        .iter()
        .any(|e| matches!(e, JournalEntry::Agent { .. }))
}

/// Did we pause Spotlight?
pub fn journal_has_spotlight() -> bool {
    read_journal()
        .iter()
        .any(|e| matches!(e, JournalEntry::Spotlight))
}

/// Is there any Apple service disabled by us?
pub fn journal_has_apple() -> bool {
    read_journal()
        .iter()
        .any(|e| matches!(e, JournalEntry::AppleAgent { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_roundtrip_both_kinds() {
        let entries = vec![
            JournalEntry::Default {
                domain: "com.apple.dock".into(),
                key: "launchanim".into(),
                present: true,
                old: "1".into(),
            },
            JournalEntry::Default {
                domain: "com.apple.universalaccess".into(),
                key: "reduceTransparency".into(),
                present: false,
                old: String::new(),
            },
            JournalEntry::Agent {
                label: "com.google.keystone.agent".into(),
                plist: "/Users/x/Library/LaunchAgents/com.google.keystone.agent.plist".into(),
            },
            JournalEntry::Spotlight,
            JournalEntry::AppleAgent {
                label: "com.apple.photoanalysisd".into(),
                uid: 501,
            },
        ];
        assert_eq!(parse_journal(&serialize_journal(&entries)), entries);
        assert!(parse_journal("junk\nmore junk").is_empty());
    }
}
