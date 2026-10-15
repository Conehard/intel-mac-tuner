//! What the menu shows, with no AppKit: text, checked items and daemon requests.
//! Testable on any system. The text comes from `i18n` according to the language.

use crate::i18n::{strings, Lang, LangPref};
use tuner::ipc::{Request, Status, TurboStatus};

/// RPM values for the "Fixed" submenu. `None` = fan maximum.
pub const FIXED_PRESETS: [Option<f64>; 5] =
    [Some(2000.0), Some(3000.0), Some(4000.0), Some(5000.0), None];

/// Temperature ranges for the "Sensor" submenu profiles (names come from `i18n`).
pub struct Profile {
    pub temp_min_c: f64,
    pub temp_max_c: f64,
}

pub const PROFILES: [Profile; 3] = [
    Profile {
        temp_min_c: 60.0,
        temp_max_c: 90.0,
    },
    Profile {
        temp_min_c: 50.0,
        temp_max_c: 85.0,
    },
    Profile {
        temp_min_c: 40.0,
        temp_max_c: 75.0,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Auto,
    Fixed(usize),
    Sensor(usize),
    SetLang(LangPref),
    ToggleTurbo,
    ToggleLogin,
    SystemVisual,
    SystemUpdaters,
    SystemSpotlight,
    SystemApple,
    SystemPurge,
    SystemUndo,
    Quit,
}

impl Action {
    /// Number stored in the NSMenuItem's `tag`.
    pub fn tag(self) -> isize {
        match self {
            Action::Auto => 1,
            Action::Fixed(i) => 10 + i as isize,
            Action::Sensor(i) => 20 + i as isize,
            Action::SetLang(LangPref::Auto) => 70,
            Action::SetLang(LangPref::Fixed(Lang::En)) => 71,
            Action::SetLang(LangPref::Fixed(Lang::Pt)) => 72,
            Action::ToggleTurbo => 80,
            Action::ToggleLogin => 90,
            Action::SystemVisual => 60,
            Action::SystemUpdaters => 61,
            Action::SystemUndo => 62,
            Action::SystemSpotlight => 63,
            Action::SystemApple => 64,
            Action::SystemPurge => 65,
            Action::Quit => 99,
        }
    }

    pub fn from_tag(tag: isize) -> Option<Action> {
        match tag {
            1 => Some(Action::Auto),
            10..=19 if ((tag - 10) as usize) < FIXED_PRESETS.len() => {
                Some(Action::Fixed((tag - 10) as usize))
            }
            20..=29 if ((tag - 20) as usize) < PROFILES.len() => {
                Some(Action::Sensor((tag - 20) as usize))
            }
            60 => Some(Action::SystemVisual),
            61 => Some(Action::SystemUpdaters),
            62 => Some(Action::SystemUndo),
            63 => Some(Action::SystemSpotlight),
            64 => Some(Action::SystemApple),
            65 => Some(Action::SystemPurge),
            70 => Some(Action::SetLang(LangPref::Auto)),
            71 => Some(Action::SetLang(LangPref::Fixed(Lang::En))),
            72 => Some(Action::SetLang(LangPref::Fixed(Lang::Pt))),
            80 => Some(Action::ToggleTurbo),
            90 => Some(Action::ToggleLogin),
            99 => Some(Action::Quit),
            _ => None,
        }
    }

    pub fn label(self, lang: Lang) -> String {
        let l = strings(lang);
        match self {
            Action::Auto => l.mode_auto.into(),
            Action::Fixed(i) => match FIXED_PRESETS[i] {
                Some(rpm) => format!("{rpm:.0} RPM"),
                None => l.maximum.into(),
            },
            Action::Sensor(i) => {
                let p = &PROFILES[i];
                format!(
                    "{} ({:.0}–{:.0} °C)",
                    l.profiles[i], p.temp_min_c, p.temp_max_c
                )
            }
            // Language names appear in their own language; "Auto" is localized.
            Action::SetLang(LangPref::Auto) => l.lang_auto.into(),
            Action::SetLang(LangPref::Fixed(Lang::En)) => "English".into(),
            Action::SetLang(LangPref::Fixed(Lang::Pt)) => "Português".into(),
            Action::ToggleTurbo => "Turbo Boost".into(),
            Action::ToggleLogin => l.start_at_login.into(),
            Action::SystemVisual => l.system_visual.into(),
            Action::SystemUpdaters => l.system_updaters.into(),
            Action::SystemSpotlight => l.system_spotlight.into(),
            Action::SystemApple => l.system_apple.into(),
            Action::SystemPurge => l.system_purge.into(),
            Action::SystemUndo => l.system_undo.into(),
            Action::Quit => l.quit.into(),
        }
    }

    /// Daemon request for this action (only the mode ones and Turbo).
    pub fn request(self, status: Option<&Status>) -> Option<Request> {
        match self {
            Action::Auto => Some(Request::SetAuto),
            Action::Fixed(i) => {
                let max = status.and_then(|s| s.fans.iter().map(|f| f.max).reduce(f64::min));
                let rpm = FIXED_PRESETS[i].or(max)?;
                Some(Request::SetFixed { rpm })
            }
            Action::Sensor(i) => {
                let p = &PROFILES[i];
                Some(Request::SetSensor {
                    temp_min_c: p.temp_min_c,
                    temp_max_c: p.temp_max_c,
                })
            }
            // Clicking Turbo flips the current state.
            Action::ToggleTurbo => match status.map(|s| s.turbo) {
                Some(TurboStatus::Enabled) => Some(Request::SetTurbo { enabled: false }),
                Some(TurboStatus::Disabled) => Some(Request::SetTurbo { enabled: true }),
                _ => None,
            },
            Action::SetLang(_)
            | Action::ToggleLogin
            | Action::SystemVisual
            | Action::SystemUpdaters
            | Action::SystemSpotlight
            | Action::SystemApple
            | Action::SystemPurge
            | Action::SystemUndo
            | Action::Quit => None,
        }
    }

    /// Should this mode item appear checked?
    pub fn is_active(self, status: &Status) -> bool {
        let close = |a: f64, b: f64| (a - b).abs() < 0.5;
        match self {
            Action::Auto => status.mode == "auto",
            Action::Fixed(i) => {
                status.mode == "fixed"
                    && match FIXED_PRESETS[i] {
                        Some(rpm) => close(status.fixed_rpm, rpm),
                        None => status.fans.iter().all(|f| status.fixed_rpm >= f.max - 0.5),
                    }
            }
            Action::Sensor(i) => {
                let p = &PROFILES[i];
                status.mode == "sensor"
                    && close(status.temp_min_c, p.temp_min_c)
                    && close(status.temp_max_c, p.temp_max_c)
            }
            Action::ToggleTurbo => status.turbo == TurboStatus::Enabled,
            Action::SetLang(_)
            | Action::ToggleLogin
            | Action::SystemVisual
            | Action::SystemUpdaters
            | Action::SystemSpotlight
            | Action::SystemApple
            | Action::SystemPurge
            | Action::SystemUndo
            | Action::Quit => false,
        }
    }
}

/// Label, whether enabled and whether checked, for the Turbo Boost item.
pub fn turbo_item(status: Option<&Status>, lang: Lang) -> (String, bool, bool) {
    let l = strings(lang);
    match status.map(|s| s.turbo) {
        Some(TurboStatus::Enabled) => (l.turbo[0].into(), true, true),
        Some(TurboStatus::Disabled) => (l.turbo[1].into(), true, false),
        Some(TurboStatus::Unmanaged) => (l.turbo[2].into(), false, false),
        Some(TurboStatus::Unknown) => (l.turbo[3].into(), false, false),
        None => (l.turbo_dash.into(), false, false),
    }
}

/// Mode items, in menu order (used in tests).
#[cfg(test)]
pub fn mode_actions() -> Vec<Action> {
    let mut v = vec![Action::Auto];
    v.extend((0..FIXED_PRESETS.len()).map(Action::Fixed));
    v.extend((0..PROFILES.len()).map(Action::Sensor));
    v
}

/// Menu bar text (language-independent).
pub fn bar_title(status: Option<&Status>) -> String {
    let Some(s) = status else {
        return "IMT —".into();
    };
    let temp = s
        .temp
        .as_ref()
        .map_or("--°".to_string(), |(_, c)| format!("{c:.0}°"));
    let rpm = s
        .fans
        .first()
        .map_or("-- rpm".to_string(), |f| format!("{:.0} rpm", f.rpm));
    let warn = if s.panic { "⚠ " } else { "" };
    format!("{warn}{temp} · {rpm}")
}

/// Informational (disabled) lines at the top of the menu. Always 3 lines.
pub fn info_lines(status: Result<&Status, &str>, lang: Lang) -> [String; 3] {
    let l = strings(lang);
    match status {
        Err(e) => [
            l.daemon_unavailable.into(),
            e.to_string(),
            l.install_hint.into(),
        ],
        Ok(s) => {
            let temp = match &s.temp {
                Some((k, c)) => format!("CPU: {c:.1} °C ({k})"),
                None => l.cpu_no_reading.into(),
            };
            let fans = if s.fans.is_empty() {
                l.fan_dash.into()
            } else {
                s.fans
                    .iter()
                    .map(|f| {
                        format!(
                            "Fan {}: {:.0} RPM ({:.0}–{:.0})",
                            f.index, f.rpm, f.min, f.max
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("  ·  ")
            };
            let mode = match s.mode.as_str() {
                "fixed" => format!(
                    "{}: {}, {:.0} RPM",
                    l.mode_label, l.mode_fixed_word, s.fixed_rpm
                ),
                "sensor" => format!(
                    "{}: {}, {} {:.0}–{:.0} °C",
                    l.mode_label, l.mode_sensor_word, s.sensor_key, s.temp_min_c, s.temp_max_c
                ),
                _ => format!("{}: {}", l.mode_label, l.mode_auto_phrase),
            };
            let mode = if s.panic {
                format!("{mode}{}", l.thermal_panic)
            } else {
                mode
            };
            [temp, fans, mode]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuner::ipc::FanStatus;

    fn st(mode: &str, fixed: f64, tmin: f64, tmax: f64) -> Status {
        Status {
            mode: mode.into(),
            fixed_rpm: fixed,
            sensor_key: "TCXC".into(),
            temp_min_c: tmin,
            temp_max_c: tmax,
            temp: Some(("TCXC".into(), 62.4)),
            panic: false,
            turbo: TurboStatus::Unmanaged,
            fans: vec![FanStatus {
                index: 0,
                rpm: 2410.0,
                min: 1200.0,
                max: 7200.0,
            }],
        }
    }

    #[test]
    fn tags_roundtrip() {
        let mut all = mode_actions();
        all.extend([
            Action::SetLang(LangPref::Auto),
            Action::SetLang(LangPref::Fixed(Lang::En)),
            Action::SetLang(LangPref::Fixed(Lang::Pt)),
            Action::ToggleTurbo,
            Action::ToggleLogin,
            Action::SystemVisual,
            Action::SystemUpdaters,
            Action::SystemSpotlight,
            Action::SystemApple,
            Action::SystemPurge,
            Action::SystemUndo,
            Action::Quit,
        ]);
        for a in all {
            assert_eq!(Action::from_tag(a.tag()), Some(a));
        }
        assert_eq!(Action::from_tag(0), None);
        assert_eq!(Action::from_tag(15), None);
        assert_eq!(Action::from_tag(23), None);
    }

    #[test]
    fn labels_translate() {
        assert_eq!(Action::Auto.label(Lang::Pt), "Automático (SMC)");
        assert_eq!(Action::Auto.label(Lang::En), "Automatic (SMC)");
        assert_eq!(Action::Fixed(0).label(Lang::En), "2000 RPM"); // RPM igual nos dois
        assert_eq!(Action::Fixed(4).label(Lang::Pt), "Máximo");
        assert_eq!(Action::Fixed(4).label(Lang::En), "Maximum");
        assert_eq!(Action::Sensor(1).label(Lang::Pt), "Equilibrado (50–85 °C)");
        assert_eq!(Action::Sensor(1).label(Lang::En), "Balanced (50–85 °C)");
        assert_eq!(Action::Quit.label(Lang::En), "Quit");
        // language names appear in their own language
        assert_eq!(
            Action::SetLang(LangPref::Fixed(Lang::Pt)).label(Lang::En),
            "Português"
        );
    }

    #[test]
    fn turbo_item_translates_and_toggles() {
        let mut s = st("auto", 3000.0, 50.0, 85.0);

        s.turbo = TurboStatus::Enabled;
        assert_eq!(turbo_item(Some(&s), Lang::Pt).0, "Turbo Boost: ligado");
        assert_eq!(turbo_item(Some(&s), Lang::En).0, "Turbo Boost: on");
        assert!(Action::ToggleTurbo.is_active(&s));
        assert_eq!(
            Action::ToggleTurbo.request(Some(&s)),
            Some(Request::SetTurbo { enabled: false })
        );

        s.turbo = TurboStatus::Disabled;
        assert_eq!(turbo_item(Some(&s), Lang::En).0, "Turbo Boost: off");
        assert_eq!(
            Action::ToggleTurbo.request(Some(&s)),
            Some(Request::SetTurbo { enabled: true })
        );

        s.turbo = TurboStatus::Unmanaged;
        assert!(!turbo_item(Some(&s), Lang::Pt).1, "unmanaged = disabled");
        assert_eq!(Action::ToggleTurbo.request(Some(&s)), None);

        s.turbo = TurboStatus::Unknown;
        assert!(!turbo_item(Some(&s), Lang::En).1, "no kext = disabled");
    }

    #[test]
    fn bar_title_formats() {
        assert_eq!(
            bar_title(Some(&st("auto", 3000.0, 50.0, 85.0))),
            "62° · 2410 rpm"
        );
        let mut p = st("auto", 3000.0, 50.0, 85.0);
        p.panic = true;
        assert!(bar_title(Some(&p)).starts_with("⚠ "));
        assert_eq!(bar_title(None), "IMT —");
    }

    #[test]
    fn requests_for_actions() {
        let s = st("auto", 3000.0, 50.0, 85.0);
        assert_eq!(
            Action::Fixed(1).request(Some(&s)),
            Some(Request::SetFixed { rpm: 3000.0 })
        );
        assert_eq!(
            Action::Fixed(4).request(Some(&s)),
            Some(Request::SetFixed { rpm: 7200.0 })
        );
        assert_eq!(Action::Fixed(4).request(None), None);
        assert_eq!(
            Action::Sensor(1).request(None),
            Some(Request::SetSensor {
                temp_min_c: 50.0,
                temp_max_c: 85.0
            })
        );
        assert_eq!(Action::Quit.request(Some(&s)), None);
        assert_eq!(Action::SetLang(LangPref::Auto).request(Some(&s)), None);
    }

    #[test]
    fn exactly_one_mode_is_checked() {
        for s in [
            st("auto", 3000.0, 50.0, 85.0),
            st("fixed", 4000.0, 50.0, 85.0),
            st("fixed", 7200.0, 50.0, 85.0),
            st("sensor", 3000.0, 60.0, 90.0),
        ] {
            let n = mode_actions()
                .into_iter()
                .filter(|a| a.is_active(&s))
                .count();
            assert_eq!(n, 1, "{:?}", (s.mode, s.fixed_rpm, s.temp_min_c));
        }
        let custom = st("fixed", 3500.0, 50.0, 85.0);
        assert_eq!(
            mode_actions()
                .into_iter()
                .filter(|a| a.is_active(&custom))
                .count(),
            0
        );
    }

    #[test]
    fn profiles_stay_below_panic_threshold() {
        for p in &PROFILES {
            assert!(p.temp_max_c < 95.0 && p.temp_max_c - p.temp_min_c >= 5.0);
        }
    }

    #[test]
    fn info_lines_translate() {
        let pt = info_lines(Ok(&st("sensor", 3000.0, 50.0, 85.0)), Lang::Pt);
        assert_eq!(pt[0], "CPU: 62.4 °C (TCXC)");
        assert!(pt[2].starts_with("Modo: sensor,"));
        let en = info_lines(Ok(&st("fixed", 3000.0, 50.0, 85.0)), Lang::En);
        assert_eq!(en[2], "Mode: fixed, 3000 RPM");
        let err = info_lines(Err("x"), Lang::En);
        assert_eq!(err[0], "Daemon unavailable");
    }
}
