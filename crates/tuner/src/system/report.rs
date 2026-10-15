//! Formatting and suggestions for the report.

use super::*;

// ---------- formatting ----------

pub(super) fn class_label(c: AgentClass) -> &'static str {
    match c {
        AgentClass::Keep => "keep",
        AgentClass::Updater => "can disable",
        AgentClass::Review => "your call",
    }
}

impl Report {
    /// User-level third-party updaters that are safe to disable.
    pub fn disableable_updaters(&self) -> Vec<&Agent> {
        self.agents
            .iter()
            .filter(|a| a.class == AgentClass::Updater && a.user_level)
            .collect()
    }

    pub fn suggestions(&self) -> Vec<String> {
        let mut s = Vec::new();
        let heavy: Vec<_> = self
            .visual
            .iter()
            .filter(|v| !v.light)
            .map(|v| v.label)
            .collect();
        if !heavy.is_empty() {
            s.push(format!(
                "Visual effects weighing on the GPU: {}. `tunerctl system trim visual` applies the light ones (reversible).",
                heavy.join(", ")
            ));
        }
        if self.spotlight_indexing == Some(true) {
            s.push("Spotlight is indexing — `tunerctl system spotlight off` pauses it (fewer CPU and disk spikes).".into());
        }
        let n = self.disableable_updaters().len();
        if n > 0 {
            s.push(format!(
                "{n} third-party updater(s) at startup — `tunerctl system trim updaters` disables them (OCLP and your own stay intact)."
            ));
        }
        let active_apple = self.apple_services.iter().filter(|a| !a.disabled).count();
        if active_apple > 0 {
            s.push(format!(
                "{active_apple} optional Apple service(s) active (photo/media analysis, Siri) — `tunerctl system trim apple` frees CPU/RAM (reversible)."
            ));
        }
        if let Some(sw) = self.memory.swap_used_bytes {
            if sw > 256 * 1024 * 1024 {
                s.push(format!(
                    "Swap in use ({}) — memory pressure; this is the top cause of stutters.",
                    human_size(sw)
                ));
            }
        }
        if let Some(fp) = self.memory.free_percent {
            if fp < 15 {
                s.push(format!("Low free memory ({fp}%) — close heavy apps."));
            }
        }
        let total: u64 = self.caches.iter().map(|c| c.bytes).sum();
        if total > 5 * 1024 * 1024 * 1024 {
            s.push(format!(
                "{} in caches (dev, trash, etc.).",
                human_size(total)
            ));
        }
        if s.is_empty() {
            s.push("Nothing glaring found. The machine is already quite lean.".into());
        }
        s
    }

    pub fn render(&self) -> String {
        let mut o = String::new();
        let line = |o: &mut String, k: &str, v: String| o.push_str(&format!("  {k:<22} {v}\n"));
        let nd = || "n/a".to_string();

        o.push_str("Intel Mac Tuner — system report (read-only)\n\n");

        let m = &self.machine;
        o.push_str("Machine\n");
        line(&mut o, "Model:", m.model.clone().unwrap_or_else(nd));
        line(&mut o, "CPU:", m.cpu.clone().unwrap_or_else(nd));
        line(
            &mut o,
            "RAM:",
            m.ram_bytes.map(human_size).unwrap_or_else(nd),
        );
        line(
            &mut o,
            "macOS:",
            match (&m.macos, &m.build) {
                (Some(v), Some(b)) => format!("{v} (build {b})"),
                (Some(v), None) => v.clone(),
                _ => nd(),
            },
        );
        line(
            &mut o,
            "OCLP:",
            match m.oclp {
                Some(true) => "yes (macOS newer than supported)".into(),
                Some(false) => "no".into(),
                None => nd(),
            },
        );

        o.push_str("\nMemory\n");
        line(
            &mut o,
            "Free memory:",
            self.memory
                .free_percent
                .map(|p| format!("{p}%"))
                .unwrap_or_else(nd),
        );
        line(
            &mut o,
            "Swap in use:",
            self.memory
                .swap_used_bytes
                .map(human_size)
                .unwrap_or_else(nd),
        );

        o.push_str("\nVisual effects (ease the GPU on an older Mac)\n");
        for v in &self.visual {
            let mark = if v.light { "ok" } else { "heavy" };
            line(
                &mut o,
                &format!("{}:", v.label),
                format!("{} [{mark}]", v.state),
            );
        }

        o.push_str("\nSpotlight\n");
        line(
            &mut o,
            "Indexing:",
            match self.spotlight_indexing {
                Some(true) => "enabled".into(),
                Some(false) => "disabled".into(),
                None => nd(),
            },
        );

        o.push_str("\nStartup items (LaunchAgents / Daemons)\n");
        let apple = self.agents.iter().filter(|a| a.apple).count();
        let thirds: Vec<_> = self.agents.iter().filter(|a| !a.apple).collect();
        line(&mut o, "Apple:", apple.to_string());
        line(&mut o, "Third-party:", thirds.len().to_string());
        for a in thirds.iter().take(25) {
            o.push_str(&format!(
                "    - {}  [{}]  ({})\n",
                a.label,
                class_label(a.class),
                a.location
            ));
        }

        if !self.apple_services.is_empty() {
            o.push_str("\nOptional Apple services (background CPU/RAM)\n");
            for s in &self.apple_services {
                let mark = if s.disabled { "disabled" } else { "active" };
                line(&mut o, &format!("{}:", s.description), format!("[{mark}]"));
            }
        }

        o.push_str("\nCaches (disk space; clearing is optional)\n");
        if self.caches.is_empty() {
            o.push_str("  (none found)\n");
        } else {
            let total: u64 = self.caches.iter().map(|c| c.bytes).sum();
            for c in &self.caches {
                line(&mut o, &format!("{}:", c.label), human_size(c.bytes));
            }
            line(&mut o, "Total listed:", human_size(total));
        }

        o.push_str("\nSuggestions\n");
        for s in self.suggestions() {
            o.push_str(&format!("  • {s}\n"));
        }
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample() -> Report {
        Report {
            machine: Machine {
                model: Some("MacBookPro14,1".into()),
                cpu: Some("Intel Core i7".into()),
                ram_bytes: Some(16 * 1024 * 1024 * 1024),
                macos: Some("15.7.3".into()),
                build: Some("24G419".into()),
                oclp: Some(true),
            },
            memory: Memory {
                free_percent: Some(84),
                swap_used_bytes: Some(0),
            },
            visual: vec![
                VisualFlag {
                    label: "Reduced transparency",
                    light: false,
                    state: "default".into(),
                },
                VisualFlag {
                    label: "Reduced motion",
                    light: true,
                    state: "on".into(),
                },
            ],
            spotlight_indexing: Some(true),
            agents: vec![
                Agent {
                    label: "com.dortania.opencore-legacy-patcher.auto-patch".into(),
                    location: "/Library/LaunchAgents",
                    apple: false,
                    class: AgentClass::Keep,
                    path: None,
                    user_level: false,
                },
                Agent {
                    label: "com.google.keystone.agent".into(),
                    location: "~/Library/LaunchAgents",
                    apple: false,
                    class: AgentClass::Updater,
                    path: Some(PathBuf::from(
                        "/Users/x/Library/LaunchAgents/com.google.keystone.agent.plist",
                    )),
                    user_level: true,
                },
            ],
            caches: vec![CacheEntry {
                label: "Xcode DerivedData",
                bytes: 6 * 1024 * 1024 * 1024,
            }],
            apple_services: vec![
                AppleService {
                    label: "com.apple.photoanalysisd",
                    description: "Photo analysis (faces, objects, memories)",
                    disabled: false,
                },
                AppleService {
                    label: "com.apple.assistantd",
                    description: "Siri",
                    disabled: true,
                },
            ],
        }
    }

    #[test]
    fn disableable_only_user_updaters() {
        let r = sample();
        let d = r.disableable_updaters();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].label, "com.google.keystone.agent");
    }

    #[test]
    fn render_and_suggestions() {
        let r = sample();
        let out = r.render();
        for s in [
            "Machine",
            "Visual effects",
            "Startup items",
            "Optional Apple services",
            "Suggestions",
        ] {
            assert!(out.contains(s), "missing {s}");
        }
        assert!(out.contains("yes (macOS newer than supported)"));
        assert!(out.contains("[keep]"));
        assert!(out.contains("[can disable]"));
        assert!(out.contains("Siri:"));
        let sug = r.suggestions().join("\n");
        assert!(sug.contains("trim visual"));
        assert!(sug.contains("trim updaters"));
        assert!(sug.contains("spotlight off"));
        assert!(sug.contains("trim apple"));
    }
}
