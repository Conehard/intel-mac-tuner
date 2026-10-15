//! The app: NSStatusItem + NSMenu, refreshed every 2 s from the daemon socket.
//! Everything runs on the main thread (an AppKit requirement). The menu is
//! rebuilt when the language changes.

use std::cell::RefCell;
use std::path::PathBuf;

use crate::i18n::{strings, Lang, LangPref};
use crate::model::{self, Action, FIXED_PRESETS, PROFILES};
use crate::objc::*;
use tuner::ipc::{self, Status};

const REFRESH_SECONDS: f64 = 2.0;
const LOGIN_LABEL: &str = "dev.intelmactuner.menu";
/// NSControlStateValueOn / Off.
const ON: isize = 1;
const OFF: isize = 0;
/// NSApplicationActivationPolicyAccessory: no Dock icon.
const POLICY_ACCESSORY: isize = 1;
/// NSVariableStatusItemLength.
const VARIABLE_LENGTH: f64 = -1.0;

/// Items whose titles/states change with the language or the status.
struct MenuRefs {
    info: [Id; 3],
    modes: Vec<(Action, Id)>,
    turbo: Id,
    login: Id,
    lang_items: Vec<(LangPref, Id)>,
    sys_visual: Id,
    sys_updaters: Id,
    sys_spotlight: Id,
    sys_apple: Id,
    sys_undo: Id,
}

struct Ui {
    app: Id,
    status_item: Id,
    handler: Id,
    button: Id,
    refs: MenuRefs,
    lang: Lang,
    pref: LangPref,
    last: Option<Status>,
    error: Option<String>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

// ---------- language: detection and persistence ----------

fn detect_system_lang() -> Lang {
    // SAFETY: NSLocale is safe to query on the main thread.
    unsafe {
        let arr = send0(class(c"NSLocale"), c"preferredLanguages");
        let first = if arr.is_null() {
            NIL
        } else {
            send0(arr, c"firstObject")
        };
        match nsstring_to_string(first) {
            Some(code) => Lang::from_locale(&code),
            None => Lang::En,
        }
    }
}

fn lang_pref_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Library/Application Support/dev.intelmactuner/ui-lang"))
}

fn load_pref() -> LangPref {
    lang_pref_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| LangPref::from_code(&s))
        .unwrap_or(LangPref::Auto)
}

fn save_pref(pref: LangPref) {
    if let Some(p) = lang_pref_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, pref.code());
    }
}

// ---------- login item ----------

fn login_agent_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(format!("Library/LaunchAgents/{LOGIN_LABEL}.plist")))
}

fn login_enabled() -> bool {
    login_agent_path().is_some_and(|p| p.exists())
}

fn set_login(enabled: bool) -> Result<(), String> {
    let path = login_agent_path().ok_or("HOME unknown")?;
    if !enabled {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("remove {}: {e}", path.display()))
            }
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe = exe
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LOGIN_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
</dict>
</plist>
"#
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, plist).map_err(|e| format!("write {}: {e}", path.display()))
}

// ---------- menu construction ----------

unsafe fn menu_item(title: &str, action: Option<Sel>, target: Id, tag: isize, enabled: bool) -> Id {
    let alloc = send0(class(c"NSMenuItem"), c"alloc");
    let item = send_id_sel_id(
        alloc,
        c"initWithTitle:action:keyEquivalent:",
        nsstring(title),
        action.unwrap_or(std::ptr::null()),
        nsstring(""),
    );
    if action.is_some() {
        send_id(item, c"setTarget:", target);
    }
    send_isize(item, c"setTag:", tag);
    send_bool(item, c"setEnabled:", Bool::from(enabled));
    item
}

unsafe fn add(menu: Id, item: Id) {
    send_id(menu, c"addItem:", item);
}

unsafe fn separator(menu: Id) {
    add(menu, send0(class(c"NSMenuItem"), c"separatorItem"));
}

unsafe fn submenu(parent_menu: Id, title: &str) -> Id {
    let parent = menu_item(title, None, NIL, 0, true);
    let sub = send_id(
        send0(class(c"NSMenu"), c"alloc"),
        c"initWithTitle:",
        nsstring(title),
    );
    send_bool(sub, c"setAutoenablesItems:", 0);
    send_id(parent, c"setSubmenu:", sub);
    add(parent_menu, parent);
    sub
}

/// Builds a fresh NSMenu in language `lang` and returns the menu + the references.
unsafe fn build_menu(handler: Id, lang: Lang) -> (Id, MenuRefs) {
    let l = strings(lang);
    let menu = new(class(c"NSMenu"));
    send_bool(menu, c"setAutoenablesItems:", 0);
    let pick = Some(sel(c"pick:"));

    let info = [(); 3].map(|_| {
        let it = menu_item("…", None, NIL, 0, false);
        add(menu, it);
        it
    });
    separator(menu);

    let mut modes = Vec::new();
    let auto = menu_item(
        &Action::Auto.label(lang),
        pick,
        handler,
        Action::Auto.tag(),
        true,
    );
    add(menu, auto);
    modes.push((Action::Auto, auto));

    let fixed = submenu(menu, l.fixed_submenu);
    for i in 0..FIXED_PRESETS.len() {
        let a = Action::Fixed(i);
        let it = menu_item(&a.label(lang), pick, handler, a.tag(), true);
        add(fixed, it);
        modes.push((a, it));
    }
    let sensor = submenu(menu, l.sensor_submenu);
    for i in 0..PROFILES.len() {
        let a = Action::Sensor(i);
        let it = menu_item(&a.label(lang), pick, handler, a.tag(), true);
        add(sensor, it);
        modes.push((a, it));
    }

    separator(menu);
    let turbo = menu_item(
        "Turbo Boost",
        pick,
        handler,
        Action::ToggleTurbo.tag(),
        false,
    );
    add(menu, turbo);

    let sys_menu = submenu(menu, l.system_submenu);
    let sys_visual = menu_item(
        &Action::SystemVisual.label(lang),
        pick,
        handler,
        Action::SystemVisual.tag(),
        true,
    );
    add(sys_menu, sys_visual);
    let sys_updaters = menu_item(
        &Action::SystemUpdaters.label(lang),
        pick,
        handler,
        Action::SystemUpdaters.tag(),
        true,
    );
    add(sys_menu, sys_updaters);
    let sys_spotlight = menu_item(
        &Action::SystemSpotlight.label(lang),
        pick,
        handler,
        Action::SystemSpotlight.tag(),
        true,
    );
    add(sys_menu, sys_spotlight);
    let sys_apple = menu_item(
        &Action::SystemApple.label(lang),
        pick,
        handler,
        Action::SystemApple.tag(),
        true,
    );
    add(sys_menu, sys_apple);
    let sys_purge = menu_item(
        &Action::SystemPurge.label(lang),
        pick,
        handler,
        Action::SystemPurge.tag(),
        true,
    );
    add(sys_menu, sys_purge);
    separator(sys_menu);
    let sys_undo = menu_item(
        &Action::SystemUndo.label(lang),
        pick,
        handler,
        Action::SystemUndo.tag(),
        false,
    );
    add(sys_menu, sys_undo);

    separator(menu);
    let lang_menu = submenu(menu, l.language);
    let mut lang_items = Vec::new();
    for pref in [
        LangPref::Auto,
        LangPref::Fixed(Lang::En),
        LangPref::Fixed(Lang::Pt),
    ] {
        let a = Action::SetLang(pref);
        let it = menu_item(&a.label(lang), pick, handler, a.tag(), true);
        add(lang_menu, it);
        lang_items.push((pref, it));
    }
    let login = menu_item(
        &Action::ToggleLogin.label(lang),
        pick,
        handler,
        Action::ToggleLogin.tag(),
        true,
    );
    add(menu, login);
    add(
        menu,
        menu_item(
            &Action::Quit.label(lang),
            pick,
            handler,
            Action::Quit.tag(),
            true,
        ),
    );

    (
        menu,
        MenuRefs {
            info,
            modes,
            turbo,
            login,
            lang_items,
            sys_visual,
            sys_updaters,
            sys_spotlight,
            sys_apple,
            sys_undo,
        },
    )
}

unsafe fn build(handler: Id, lang: Lang, pref: LangPref) -> Ui {
    let app = send0(class(c"NSApplication"), c"sharedApplication");
    send_isize(app, c"setActivationPolicy:", POLICY_ACCESSORY);

    let bar = send0(class(c"NSStatusBar"), c"systemStatusBar");
    let status_item = send0(
        send_f64(bar, c"statusItemWithLength:", VARIABLE_LENGTH),
        c"retain",
    );
    let button = send0(status_item, c"button");
    let font = send_f64_f64(
        class(c"NSFont"),
        c"monospacedDigitSystemFontOfSize:weight:",
        0.0,
        0.0,
    );
    if !font.is_null() {
        send_id(button, c"setFont:", font);
    }
    send_id(button, c"setTitle:", nsstring("IMT …"));

    let (menu, refs) = build_menu(handler, lang);
    send_id(status_item, c"setMenu:", menu);
    Ui {
        app,
        status_item,
        handler,
        button,
        refs,
        lang,
        pref,
        last: None,
        error: None,
    }
}

/// Switches the language, saves the preference and rebuilds the menu.
fn change_lang(pref: LangPref) {
    UI.with(|cell| {
        let mut guard = cell.borrow_mut();
        let Some(ui) = guard.as_mut() else { return };
        ui.pref = pref;
        ui.lang = pref.resolve(detect_system_lang());
        save_pref(pref);
        // SAFETY: main thread; replaces the status item's menu.
        unsafe {
            let (menu, refs) = build_menu(ui.handler, ui.lang);
            send_id(ui.status_item, c"setMenu:", menu);
            ui.refs = refs;
        }
    });
    refresh();
}

// ---------- refresh ----------

fn refresh() {
    let result = ipc::status();
    UI.with(|cell| {
        let mut guard = cell.borrow_mut();
        let Some(ui) = guard.as_mut() else { return };
        match result {
            Ok(st) => ui.last = Some(st),
            Err(e) => {
                ui.last = None;
                ui.error = Some(e);
            }
        }
        let lang = ui.lang;
        let lines = match (&ui.last, &ui.error) {
            (Some(st), None) => model::info_lines(Ok(st), lang),
            (Some(st), Some(err)) => {
                let mut l = model::info_lines(Ok(st), lang);
                l[2] = format!("{}: {err}", strings(lang).error);
                l
            }
            (None, err) => model::info_lines(Err(err.as_deref().unwrap_or("—")), lang),
        };
        // SAFETY: AppKit objects created in `build`/`build_menu`, used on the main thread.
        unsafe {
            send_id(
                ui.button,
                c"setTitle:",
                nsstring(&model::bar_title(ui.last.as_ref())),
            );
            for (item, text) in ui.refs.info.iter().zip(lines.iter()) {
                send_id(*item, c"setTitle:", nsstring(text));
            }
            let reachable = ui.last.is_some();
            for (action, item) in &ui.refs.modes {
                let on = ui.last.as_ref().is_some_and(|st| action.is_active(st));
                send_isize(*item, c"setState:", if on { ON } else { OFF });
                send_bool(*item, c"setEnabled:", Bool::from(reachable));
            }
            let (turbo_title, turbo_enabled, turbo_checked) =
                model::turbo_item(ui.last.as_ref(), lang);
            send_id(ui.refs.turbo, c"setTitle:", nsstring(&turbo_title));
            send_isize(
                ui.refs.turbo,
                c"setState:",
                if turbo_checked { ON } else { OFF },
            );
            send_bool(ui.refs.turbo, c"setEnabled:", Bool::from(turbo_enabled));
            for (pref, item) in &ui.refs.lang_items {
                send_isize(*item, c"setState:", if *pref == ui.pref { ON } else { OFF });
            }
            send_isize(
                ui.refs.login,
                c"setState:",
                if login_enabled() { ON } else { OFF },
            );
            let visual_on = tuner::system::journal_has_visual();
            let updaters_on = tuner::system::journal_has_agents();
            let spotlight_on = tuner::system::journal_has_spotlight();
            let apple_on = tuner::system::journal_has_apple();
            send_isize(
                ui.refs.sys_visual,
                c"setState:",
                if visual_on { ON } else { OFF },
            );
            send_isize(
                ui.refs.sys_updaters,
                c"setState:",
                if updaters_on { ON } else { OFF },
            );
            send_isize(
                ui.refs.sys_spotlight,
                c"setState:",
                if spotlight_on { ON } else { OFF },
            );
            send_isize(
                ui.refs.sys_apple,
                c"setState:",
                if apple_on { ON } else { OFF },
            );
            send_bool(
                ui.refs.sys_undo,
                c"setEnabled:",
                Bool::from(visual_on || updaters_on || spotlight_on || apple_on),
            );
        }
        if ui.last.is_some() {
            ui.error = None;
        }
    });
}

fn act(tag: isize) {
    let Some(action) = Action::from_tag(tag) else {
        return;
    };
    match action {
        Action::Quit => {
            UI.with(|cell| {
                if let Some(ui) = cell.borrow().as_ref() {
                    // SAFETY: valid NSApp on the main thread.
                    unsafe { send_id(ui.app, c"terminate:", NIL) };
                }
            });
            return;
        }
        Action::SetLang(pref) => {
            change_lang(pref);
            return;
        }
        Action::ToggleLogin => {
            if let Err(e) = set_login(!login_enabled()) {
                set_error(e);
            }
        }
        Action::SystemVisual => {
            let r = if tuner::system::journal_has_visual() {
                tuner::system::restore_visual().map(|_| ())
            } else {
                tuner::system::trim_visual().map(|_| ())
            };
            if let Err(e) = r {
                set_error(e);
            }
        }
        Action::SystemUpdaters => {
            let r = if tuner::system::journal_has_agents() {
                tuner::system::restore_updaters().map(|_| ())
            } else {
                tuner::system::trim_updaters().map(|_| ())
            };
            if let Err(e) = r {
                set_error(e);
            }
        }
        Action::SystemSpotlight => {
            let r = if tuner::system::journal_has_spotlight() {
                tuner::system::resume_spotlight().map(|_| ())
            } else {
                tuner::system::pause_spotlight()
            };
            if let Err(e) = r {
                set_error(e);
            }
        }
        Action::SystemApple => {
            let r = if tuner::system::journal_has_apple() {
                tuner::system::restore_apple_services().map(|_| ())
            } else {
                tuner::system::disable_apple_services().map(|_| ())
            };
            if let Err(e) = r {
                set_error(e);
            }
        }
        Action::SystemPurge => {
            if let Err(e) = tuner::system::purge_memory() {
                set_error(e);
            }
        }
        Action::SystemUndo => {
            if let Err(e) = tuner::system::restore() {
                set_error(e);
            }
        }
        _ => {
            let last = UI.with(|cell| cell.borrow().as_ref().and_then(|ui| ui.last.clone()));
            let outcome = match action.request(last.as_ref()) {
                Some(req) => ipc::set(&req),
                None => Ok(()),
            };
            if let Err(e) = outcome {
                set_error(e);
            }
        }
    }
    refresh();
}

fn set_error(e: String) {
    UI.with(|cell| {
        if let Some(ui) = cell.borrow_mut().as_mut() {
            ui.error = Some(e);
        }
    });
}

// ---------- callbacks Objective-C ----------

fn guarded(f: impl FnOnce()) {
    // A panic crossing the C boundary would abort the app.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| with_pool(f)));
}

extern "C" fn on_tick(_this: Id, _cmd: Sel, _timer: Id) {
    guarded(refresh);
}

extern "C" fn on_pick(_this: Id, _cmd: Sel, sender: Id) {
    guarded(|| {
        // SAFETY: `sender` is the clicked NSMenuItem.
        let tag = unsafe { send_ret_isize(sender, c"tag") };
        act(tag);
    });
}

pub fn run() {
    with_pool(|| {
        let cls = define_class(
            c"IMTMenuHandler",
            &[(c"tick:", on_tick), (c"pick:", on_pick)],
        );
        let handler = new(cls);
        let pref = load_pref();
        let lang = pref.resolve(detect_system_lang());
        // SAFETY: all on the main thread, before the run loop.
        let app = unsafe {
            let ui = build(handler, lang, pref);
            let app = ui.app;
            UI.with(|cell| *cell.borrow_mut() = Some(ui));
            send_timer(
                class(c"NSTimer"),
                c"scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:",
                REFRESH_SECONDS,
                handler,
                sel(c"tick:"),
                NIL,
                1,
            );
            app
        };
        refresh();
        // SAFETY: enters the AppKit run loop; only returns on quit.
        unsafe { send0(app, c"run") };
    });
}
