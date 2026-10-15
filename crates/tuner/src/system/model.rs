//! Data model of the system report.

use std::path::PathBuf;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Machine {
    pub model: Option<String>,
    pub cpu: Option<String>,
    pub ram_bytes: Option<u64>,
    pub macos: Option<String>,
    pub build: Option<String>,
    pub oclp: Option<bool>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Memory {
    pub free_percent: Option<u32>,
    pub swap_used_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisualFlag {
    pub label: &'static str,
    pub light: bool,
    pub state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentClass {
    /// Critical or the user's own: never suggest disabling.
    Keep,
    /// Updater/bloat: safe to disable.
    Updater,
    /// The user decides.
    Review,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    pub label: String,
    pub location: &'static str,
    pub apple: bool,
    pub class: AgentClass,
    /// Path to the .plist, when known.
    pub path: Option<PathBuf>,
    /// Is it a user agent (`~/Library/LaunchAgents`)? Only those are touched without root.
    pub user_level: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntry {
    pub label: &'static str,
    pub bytes: u64,
}

/// Optional Apple service (photo/media analysis, Siri…) present on the machine.
#[derive(Debug, Clone, PartialEq)]
pub struct AppleService {
    pub label: &'static str,
    pub description: &'static str,
    /// Disabled by us (recorded in the journal)?
    pub disabled: bool,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    pub machine: Machine,
    pub memory: Memory,
    pub visual: Vec<VisualFlag>,
    pub spotlight_indexing: Option<bool>,
    pub agents: Vec<Agent>,
    pub caches: Vec<CacheEntry>,
    pub apple_services: Vec<AppleService>,
}
