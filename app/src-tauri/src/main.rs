// Suppress the extra console window on Windows. Harmless on macOS, and keeps the
// crate honest if it is ever cross-checked on another target.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    woosh_app_lib::run();
}
