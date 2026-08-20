// Release builds are GUI-subsystem binaries: without this, launching
// `ide.exe` on Windows opens a console window first and leaves it behind the
// IDE for the whole session. Debug builds keep the console so `println!` and
// panic output stay visible while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    let exit_code = ui_shell::run_app();
    ExitCode::from(exit_code as u8)
}
