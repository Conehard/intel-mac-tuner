//! Pure helpers: sizes, agent classification, parsing of command output.

use super::*;

// ---------- pure helpers (tested) ----------

pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

pub fn is_apple_label(label: &str) -> bool {
    label.starts_with("com.apple.")
}

pub fn classify_agent(label: &str) -> AgentClass {
    let l = label.to_ascii_lowercase();
    const KEEP: &[&str] = &[
        "com.dortania.",
        "dev.intelmactuner.",
        "com.nordvpn.",
        "com.wireguard.",
        "com.objective-see.",
        "com.paloaltonetworks.",
    ];
    if KEEP.iter().any(|p| l.starts_with(p)) {
        return AgentClass::Keep;
    }
    let is_updater = l.contains("keystone")
        || l.contains("googleupdater")
        || l.contains("com.microsoft.autoupdate")
        || l.contains("com.microsoft.update")
        || (l.contains("adobe") && l.contains("update"))
        || (l.contains("dropbox") && l.contains("update"))
        || l.contains("org.sparkle-project")
        || l.contains("updater")
        || l.contains(".update.");
    if is_updater {
        AgentClass::Updater
    } else {
        AgentClass::Review
    }
}

pub fn parse_swap_used(line: &str) -> Option<u64> {
    let after = line.split("used =").nth(1)?.trim();
    let token = after.split_whitespace().next()?;
    parse_size_token(token)
}

pub(super) fn parse_size_token(tok: &str) -> Option<u64> {
    let tok = tok.trim();
    let (num, mult) = match tok.chars().last()? {
        'K' | 'k' => (&tok[..tok.len() - 1], 1024.0),
        'M' | 'm' => (&tok[..tok.len() - 1], 1024.0 * 1024.0),
        'G' | 'g' => (&tok[..tok.len() - 1], 1024.0 * 1024.0 * 1024.0),
        'B' | 'b' => (&tok[..tok.len() - 1], 1.0),
        c if c.is_ascii_digit() => (tok, 1.0),
        _ => return None,
    };
    let num: f64 = num.replace(',', ".").parse().ok()?;
    Some((num * mult) as u64)
}

pub fn parse_free_percent(text: &str) -> Option<u32> {
    let line = text
        .lines()
        .find(|l| l.to_lowercase().contains("free percentage"))?;
    let n: String = line.chars().filter(|c| c.is_ascii_digit()).collect();
    n.parse().ok().filter(|p| *p <= 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn agent_classification() {
        for keep in [
            "com.dortania.opencore-legacy-patcher.auto-patch",
            "dev.intelmactuner.daemon",
            "com.wireguard.macos",
            "com.nordvpn.macos.helper",
        ] {
            assert_eq!(classify_agent(keep), AgentClass::Keep, "{keep}");
        }
        for upd in [
            "com.google.keystone.agent",
            "com.google.GoogleUpdater.wake",
            "com.microsoft.autoupdate.helper",
            "org.sparkle-project.Sparkle.Autoupdate",
        ] {
            assert_eq!(classify_agent(upd), AgentClass::Updater, "{upd}");
        }
        assert_eq!(classify_agent("com.someapp.helper"), AgentClass::Review);
    }

    #[test]
    fn swap_and_percent() {
        assert_eq!(
            parse_swap_used("total = 2048,00M  used = 512,00M  free = 1536,00M"),
            Some(512 * 1024 * 1024)
        );
        assert_eq!(
            parse_free_percent("System-wide memory free percentage: 72%"),
            Some(72)
        );
    }
}
