//! The setup program: installs what Mixer needs, next to it, and nothing
//! else (spec section 6). Windows only; elsewhere it explains that and stops.
//!
//! The development machine cannot build for Windows, so the Windows code is
//! type-checked there with `RUSTFLAGS="--cfg check_windows" cargo check`.

// No console window behind the setup window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Elsewhere, only the tests use the shared parts.
#![cfg_attr(not(any(windows, check_windows)), allow(dead_code))]

#[cfg(any(windows, check_windows))]
mod checks;
mod core;
#[cfg(any(windows, check_windows))]
mod gui;
#[cfg(any(windows, check_windows))]
mod net;

fn main() {
    // For automated checks: install without a window and report by exit code.
    #[cfg(any(windows, check_windows))]
    if std::env::args().any(|a| a == "--no-window") {
        std::process::exit(gui::run_without_window());
    }
    #[cfg(any(windows, check_windows))]
    gui::run();
    #[cfg(not(any(windows, check_windows)))]
    println!("Mixer setup runs on Windows only.");
}
