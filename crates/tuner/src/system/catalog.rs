//! Fixed tables: visual-effect targets and the allowlist of optional Apple services.

use std::path::Path;

// ---------- visual effects: single table (report + trim) ----------

pub(super) struct VisualTarget {
    pub(super) label: &'static str,
    pub(super) domain: &'static str,
    pub(super) key: &'static str,
    pub(super) light_value: bool,
    pub(super) dock: bool,
}

pub(super) const VISUAL_TARGETS: &[VisualTarget] = &[
    VisualTarget {
        label: "Reduced transparency",
        domain: "com.apple.universalaccess",
        key: "reduceTransparency",
        light_value: true,
        dock: false,
    },
    VisualTarget {
        label: "Reduced motion",
        domain: "com.apple.universalaccess",
        key: "reduceMotion",
        light_value: true,
        dock: false,
    },
    VisualTarget {
        label: "Window animations",
        domain: "NSGlobalDomain",
        key: "NSAutomaticWindowAnimationsEnabled",
        light_value: false,
        dock: false,
    },
    VisualTarget {
        label: "App launch animation (Dock)",
        domain: "com.apple.dock",
        key: "launchanim",
        light_value: false,
        dock: true,
    },
];

// ---------- optional Apple services (fixed list, for safety) ----------

/// Heavy, optional Apple user agents, safe to disable and reversible. The daemon
/// only accepts labels from this list. They are `gui/<uid>` agents; presence is
/// checked by the .plist in `/System/Library/LaunchAgents`.
pub const APPLE_OPTIONAL: &[(&str, &str)] = &[
    (
        "com.apple.photoanalysisd",
        "Photo analysis (faces, objects, memories)",
    ),
    (
        "com.apple.mediaanalysisd",
        "Media analysis (Live Text, Visual Look Up)",
    ),
    ("com.apple.parsecd", "Siri and Spotlight suggestions"),
    ("com.apple.assistantd", "Siri"),
    ("com.apple.suggestd", "Proactive suggestions and learning"),
];

/// Is the label in the list of Apple agents we allow touching?
pub fn is_allowed_apple_agent(label: &str) -> bool {
    APPLE_OPTIONAL.iter().any(|(l, _)| *l == label)
}

pub(super) fn apple_agent_present(label: &str) -> bool {
    Path::new("/System/Library/LaunchAgents")
        .join(format!("{label}.plist"))
        .exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_allowlist_guards_daemon() {
        assert!(is_allowed_apple_agent("com.apple.photoanalysisd"));
        assert!(is_allowed_apple_agent("com.apple.assistantd"));
        assert!(!is_allowed_apple_agent("com.apple.WindowServer"));
        assert!(!is_allowed_apple_agent(
            "com.dortania.opencore-legacy-patcher"
        ));
        assert!(!is_allowed_apple_agent(""));
    }
}
