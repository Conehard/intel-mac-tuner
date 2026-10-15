//! Intel Mac Tuner — menu bar. Pure Rust over the Objective-C runtime: no Swift, no Xcode.

// Outside macOS the AppKit code is only compiled for type-checking.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod i18n;
mod model;

#[cfg(any(target_os = "macos", feature = "check-ffi"))]
mod app;
#[cfg(any(target_os = "macos", feature = "check-ffi"))]
mod objc;

#[cfg(target_os = "macos")]
fn main() {
    app::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("intel-mac-tuner-menu only runs on macOS");
    std::process::exit(1);
}
