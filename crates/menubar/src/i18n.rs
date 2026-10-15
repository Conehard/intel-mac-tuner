//! Menu text in English and Portuguese. Pure and testable (no AppKit).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Pt,
}

/// Saved language preference: automatic (follows the system) or fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangPref {
    Auto,
    Fixed(Lang),
}

impl Lang {
    /// Infers the language from a locale code (e.g. "pt-BR", "en_US").
    pub fn from_locale(code: &str) -> Lang {
        if code.to_ascii_lowercase().starts_with("pt") {
            Lang::Pt
        } else {
            Lang::En
        }
    }
}

impl LangPref {
    pub fn code(self) -> &'static str {
        match self {
            LangPref::Auto => "auto",
            LangPref::Fixed(Lang::En) => "en",
            LangPref::Fixed(Lang::Pt) => "pt",
        }
    }

    pub fn from_code(s: &str) -> LangPref {
        match s.trim() {
            "en" => LangPref::Fixed(Lang::En),
            "pt" => LangPref::Fixed(Lang::Pt),
            _ => LangPref::Auto,
        }
    }

    /// Resolves to a concrete language; `system` is used when the preference is Auto.
    pub fn resolve(self, system: Lang) -> Lang {
        match self {
            LangPref::Auto => system,
            LangPref::Fixed(l) => l,
        }
    }
}

/// Text table for one language.
pub struct L {
    pub mode_auto: &'static str,
    pub maximum: &'static str,
    pub fixed_submenu: &'static str,
    pub sensor_submenu: &'static str,
    pub profiles: [&'static str; 3],
    pub turbo: [&'static str; 4], // on, off, unmanaged, no_kext
    pub turbo_dash: &'static str,
    pub start_at_login: &'static str,
    pub quit: &'static str,
    pub language: &'static str,
    pub lang_auto: &'static str,
    pub system_submenu: &'static str,
    pub system_visual: &'static str,
    pub system_updaters: &'static str,
    pub system_spotlight: &'static str,
    pub system_apple: &'static str,
    pub system_purge: &'static str,
    pub system_undo: &'static str,
    pub daemon_unavailable: &'static str,
    pub install_hint: &'static str,
    pub cpu_no_reading: &'static str,
    pub fan_dash: &'static str,
    pub mode_label: &'static str,
    pub mode_fixed_word: &'static str,
    pub mode_sensor_word: &'static str,
    pub mode_auto_phrase: &'static str,
    pub thermal_panic: &'static str,
    pub error: &'static str,
}

pub const EN: L = L {
    mode_auto: "Automatic (SMC)",
    maximum: "Maximum",
    fixed_submenu: "Fixed speed",
    sensor_submenu: "By temperature",
    profiles: ["Quiet", "Balanced", "Cool"],
    turbo: [
        "Turbo Boost: on",
        "Turbo Boost: off",
        "Turbo Boost: not managed",
        "Turbo Boost: kext not installed",
    ],
    turbo_dash: "Turbo Boost: —",
    start_at_login: "Start at login",
    quit: "Quit",
    language: "Language",
    lang_auto: "Automatic",
    system_submenu: "Optimize system",
    system_visual: "Lighten visual effects",
    system_updaters: "Disable third-party updaters",
    system_spotlight: "Pause Spotlight indexing",
    system_apple: "Disable optional Apple services",
    system_purge: "Free memory now",
    system_undo: "Undo optimizations",
    daemon_unavailable: "Daemon unavailable",
    install_hint: "Install with: sudo tunerctl install",
    cpu_no_reading: "CPU: no reading",
    fan_dash: "Fan: —",
    mode_label: "Mode",
    mode_fixed_word: "fixed",
    mode_sensor_word: "sensor",
    mode_auto_phrase: "automatic (SMC)",
    thermal_panic: " — THERMAL PANIC, fans at max",
    error: "Error",
};

pub const PT: L = L {
    mode_auto: "Automático (SMC)",
    maximum: "Máximo",
    fixed_submenu: "Rotação fixa",
    sensor_submenu: "Por temperatura",
    profiles: ["Silencioso", "Equilibrado", "Frio"],
    turbo: [
        "Turbo Boost: ligado",
        "Turbo Boost: desligado",
        "Turbo Boost: não gerenciado",
        "Turbo Boost: kext não instalado",
    ],
    turbo_dash: "Turbo Boost: —",
    start_at_login: "Iniciar com o Mac",
    quit: "Sair",
    language: "Idioma",
    lang_auto: "Automático",
    system_submenu: "Otimizar o sistema",
    system_visual: "Aliviar efeitos visuais",
    system_updaters: "Desativar atualizadores de terceiros",
    system_spotlight: "Pausar indexação do Spotlight",
    system_apple: "Desativar serviços opcionais da Apple",
    system_purge: "Liberar memória agora",
    system_undo: "Desfazer otimizações",
    daemon_unavailable: "Daemon indisponível",
    install_hint: "Instale com: sudo tunerctl install",
    cpu_no_reading: "CPU: sem leitura",
    fan_dash: "Fan: —",
    mode_label: "Modo",
    mode_fixed_word: "fixo",
    mode_sensor_word: "sensor",
    mode_auto_phrase: "automático (SMC)",
    thermal_panic: " — PÂNICO TÉRMICO, fans no máximo",
    error: "Erro",
};

pub fn strings(lang: Lang) -> &'static L {
    match lang {
        Lang::En => &EN,
        Lang::Pt => &PT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_detection() {
        assert_eq!(Lang::from_locale("pt-BR"), Lang::Pt);
        assert_eq!(Lang::from_locale("pt_PT.UTF-8"), Lang::Pt);
        assert_eq!(Lang::from_locale("en-US"), Lang::En);
        assert_eq!(Lang::from_locale("fr"), Lang::En); // fallback
    }

    #[test]
    fn pref_roundtrip_and_resolve() {
        for p in [
            LangPref::Auto,
            LangPref::Fixed(Lang::En),
            LangPref::Fixed(Lang::Pt),
        ] {
            assert_eq!(LangPref::from_code(p.code()), p);
        }
        assert_eq!(LangPref::from_code("xx"), LangPref::Auto);
        assert_eq!(LangPref::Auto.resolve(Lang::Pt), Lang::Pt);
        assert_eq!(LangPref::Fixed(Lang::En).resolve(Lang::Pt), Lang::En);
    }

    #[test]
    fn both_tables_filled() {
        for l in [strings(Lang::En), strings(Lang::Pt)] {
            assert!(!l.mode_auto.is_empty() && !l.quit.is_empty());
            assert_eq!(l.profiles.len(), 3);
            assert_eq!(l.turbo.len(), 4);
        }
    }
}
