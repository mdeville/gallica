use std::process::ExitCode;

fn main() -> ExitCode {
    gallica_tui::main_with(std::env::args_os())
}
