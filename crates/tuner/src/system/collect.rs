//! Read-only collection of the machine state.

use super::*;
use std::path::{Path, PathBuf};

// ---------- collection ----------

pub(super) fn collect_machine(agents: &[Agent]) -> Machine {
    let oclp_by_agents = agents
        .iter()
        .any(|a| a.label.contains("opencore-legacy-patcher"));
    let oclp = if oclp_by_agents {
        Some(true)
    } else if let Some(nv) = sh("/usr/sbin/nvram", &["-p"]) {
        let low = nv.to_lowercase();
        Some(low.contains("oclp") || low.contains("opencore-legacy"))
    } else {
        None
    };
    Machine {
        model: sysctl("hw.model"),
        cpu: sysctl("machdep.cpu.brand_string"),
        ram_bytes: sysctl("hw.memsize").and_then(|s| s.parse().ok()),
        macos: sysctl("kern.osproductversion"),
        build: sysctl("kern.osversion"),
        oclp,
    }
}

pub(super) fn collect_memory() -> Memory {
    Memory {
        swap_used_bytes: sysctl("vm.swapusage").and_then(|l| parse_swap_used(&l)),
        free_percent: sh("/usr/bin/memory_pressure", &["-Q"]).and_then(|t| parse_free_percent(&t)),
    }
}

pub(super) fn collect_visual() -> Vec<VisualFlag> {
    VISUAL_TARGETS
        .iter()
        .map(|t| {
            let raw = defaults_read(t.domain, t.key);
            let (light, state) = match raw.as_deref() {
                Some("1") => (t.light_value, "on".to_string()),
                Some("0") => (!t.light_value, "off".to_string()),
                Some(v) => (false, v.to_string()),
                None => (false, "default".to_string()),
            };
            VisualFlag {
                label: t.label,
                light,
                state,
            }
        })
        .collect()
}

pub(super) fn collect_spotlight() -> Option<bool> {
    let out = sh("/usr/bin/mdutil", &["-s", "/"])?;
    let low = out.to_lowercase();
    if low.contains("enabled") {
        Some(true)
    } else if low.contains("disabled") {
        Some(false)
    } else {
        None
    }
}

pub(super) fn collect_agents() -> Vec<Agent> {
    let h = home();
    let dirs: [(PathBuf, &'static str, bool); 3] = [
        (
            h.join("Library/LaunchAgents"),
            "~/Library/LaunchAgents",
            true,
        ),
        (
            PathBuf::from("/Library/LaunchAgents"),
            "/Library/LaunchAgents",
            false,
        ),
        (
            PathBuf::from("/Library/LaunchDaemons"),
            "/Library/LaunchDaemons",
            false,
        ),
    ];
    let mut agents = Vec::new();
    for (dir, loc, user_level) in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(label) = name.strip_suffix(".plist") else {
                continue;
            };
            let apple = is_apple_label(label);
            agents.push(Agent {
                label: label.to_string(),
                location: loc,
                apple,
                class: if apple {
                    AgentClass::Keep
                } else {
                    classify_agent(label)
                },
                path: Some(e.path()),
                user_level,
            });
        }
    }
    agents.sort_by(|a, b| a.label.cmp(&b.label));
    agents
}

pub(super) fn dir_size_bytes(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    let out = sh("/usr/bin/du", &["-sk", &path.to_string_lossy()])?;
    let kb: u64 = out.split_whitespace().next()?.parse().ok()?;
    Some(kb * 1024)
}

pub(super) fn collect_caches() -> Vec<CacheEntry> {
    let h = home();
    let specs: &[(&'static str, &str)] = &[
        ("Xcode DerivedData", "Library/Developer/Xcode/DerivedData"),
        (
            "Xcode iOS DeviceSupport",
            "Library/Developer/Xcode/iOS DeviceSupport",
        ),
        ("Xcode Archives", "Library/Developer/Xcode/Archives"),
        (
            "CoreSimulator (caches)",
            "Library/Developer/CoreSimulator/Caches",
        ),
        ("User caches", "Library/Caches"),
        ("Gradle", ".gradle/caches"),
        ("npm", ".npm"),
        ("CocoaPods", ".cocoapods/repos"),
        ("pip", "Library/Caches/pip"),
        ("Logs", "Library/Logs"),
        ("Trash", ".Trash"),
    ];
    let mut out = Vec::new();
    for (label, rel) in specs {
        if let Some(bytes) = dir_size_bytes(&h.join(rel)) {
            if bytes > 0 {
                out.push(CacheEntry { label, bytes });
            }
        }
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.bytes));
    out
}

pub(super) fn collect_apple_services() -> Vec<AppleService> {
    let disabled: Vec<String> = read_journal()
        .into_iter()
        .filter_map(|e| match e {
            JournalEntry::AppleAgent { label, .. } => Some(label),
            _ => None,
        })
        .collect();
    APPLE_OPTIONAL
        .iter()
        .filter(|(label, _)| apple_agent_present(label))
        .map(|(label, description)| AppleService {
            label,
            description,
            disabled: disabled.iter().any(|d| d == label),
        })
        .collect()
}

pub fn collect() -> Report {
    let agents = collect_agents();
    Report {
        machine: collect_machine(&agents),
        memory: collect_memory(),
        visual: collect_visual(),
        spotlight_indexing: collect_spotlight(),
        agents,
        caches: collect_caches(),
        apple_services: collect_apple_services(),
    }
}
