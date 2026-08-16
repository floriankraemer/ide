use std::process::ExitCode;

fn main() -> ExitCode {
    let exit_code = ui_shell::run_app();
    ExitCode::from(exit_code as u8)
}
