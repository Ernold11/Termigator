// Windows release builds are GUI programs: no console window next to the app from the Start menu.
// The askpass helper (the same exe) still answers ssh, which hands it pipes for its output.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    termigator::run()
}
