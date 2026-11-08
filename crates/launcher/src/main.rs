//! The Mixer Launcher: starts Mixer, updates it from GitHub Releases when asked, and
//! installs what it needs next to it, and nothing else (spec section 6). Windows only;
//! elsewhere it explains that and stops.
//!
//! The development machine cannot build for Windows, so the Windows code is
//! type-checked there with `RUSTFLAGS="--cfg check_windows" cargo check`.

// No console window behind the launcher window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Elsewhere, only the tests use the shared parts.
#![cfg_attr(not(any(windows, check_windows)), allow(dead_code))]

#[cfg(any(windows, check_windows))]
mod checks;
mod core;
#[cfg(any(windows, check_windows))]
mod gui;
mod jobs;
mod json;
#[cfg(any(windows, check_windows))]
mod net;
mod settings;
mod update;
mod view;

fn main() {
    // For automated checks: install without a window and report by exit code.
    #[cfg(any(windows, check_windows))]
    if std::env::args().any(|a| a == "--no-window") {
        std::process::exit(gui::run_without_window());
    }
    #[cfg(any(windows, check_windows))]
    gui::run();
    #[cfg(not(any(windows, check_windows)))]
    println!("The Mixer Launcher runs on Windows only.");
}
