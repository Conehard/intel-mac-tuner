//! `tunerctl system` — thin layer over `tuner::system`.
//!
//! The logic (collection, classification, journal, trim/restore) lives in the
//! `tuner` lib so the menu bar can use the same thing. Here we just dispatch and print.

use tuner::system;

/// `tunerctl system [report | trim visual | trim updaters | trim apple | spotlight on|off | purge | restore]`
pub fn command(args: &[&str]) -> Result<(), String> {
    match args {
        [] | ["report"] => {
            print!("{}", system::collect().render());
            Ok(())
        }
        ["trim", "visual"] => {
            let out = system::trim_visual()?;
            if out.changed == 0 {
                println!("Visual effects were already in the light mode. Nothing to do.");
            } else {
                println!(
                    "{} visual effect(s) set to light mode. Undo: tunerctl system restore",
                    out.changed
                );
                if out.needs_relogin {
                    println!("Reduced transparency only fully applies after you log out and back in (or reboot).");
                }
            }
            Ok(())
        }
        ["trim", "updaters"] => {
            let done = system::trim_updaters()?;
            if done.is_empty() {
                println!("No third-party updaters to disable. Nothing to do.");
            } else {
                println!(
                    "{} updater(s) disabled. Undo: tunerctl system restore",
                    done.len()
                );
                for label in &done {
                    println!("    - {label}");
                }
                println!("OCLP, Apple and your own agents were left intact.");
            }
            Ok(())
        }
        ["trim", "apple"] => {
            let done = system::disable_apple_services()?;
            if done.is_empty() {
                println!("No optional Apple services to disable. Nothing to do.");
            } else {
                println!(
                    "{} Apple service(s) disabled. Undo: tunerctl system restore",
                    done.len()
                );
                for label in &done {
                    println!("    - {label}");
                }
                println!("Re-enable at any time. Siri/photo analysis come back when re-enabled.");
            }
            Ok(())
        }
        ["spotlight", "off"] => {
            system::pause_spotlight()?;
            println!("Spotlight paused. Resume: tunerctl system spotlight on");
            Ok(())
        }
        ["spotlight", "on"] => {
            let n = system::resume_spotlight()?;
            if n == 0 {
                println!("Spotlight was not paused by us. Nothing to do.");
            } else {
                println!("Spotlight resumed (it will reindex in the background).");
            }
            Ok(())
        }
        ["purge"] => {
            system::purge_memory()?;
            println!("Inactive memory freed.");
            Ok(())
        }
        ["restore"] => {
            let n = system::restore()?;
            if n == 0 {
                println!("Nothing to undo (empty journal).");
            } else {
                println!("{n} change(s) undone. Previous state restored.");
                println!("Transparency fully returns after you log out and back in.");
            }
            Ok(())
        }
        _ => Err("usage: tunerctl system [report | trim visual | trim updaters | trim apple | spotlight on|off | purge | restore]".into()),
    }
}
