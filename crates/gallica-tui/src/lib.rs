//! The `gallica` binary: headless commands (`info`, `plan`, `download`) and
//! the terminal UI (`tui`).

pub mod cli;
pub mod headless;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command};

/// Exit code when some items failed.
pub const EXIT_PARTIAL: u8 = 1;
/// Exit code for bad arguments or configuration.
pub const EXIT_USAGE: u8 = 2;
/// Exit code when Gallica refuses automated access.
pub const EXIT_BLOCKED: u8 = 3;

/// Parses the arguments and runs.
pub fn main_with<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::from(EXIT_USAGE)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    init_tracing(cli.verbose);
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("cannot start the async runtime: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let code = match cli.command.clone() {
        Some(Command::Info { target }) => {
            runtime.block_on(headless::run(&cli, headless::Mode::Info, &target))
        }
        Some(Command::Plan { target }) => {
            runtime.block_on(headless::run(&cli, headless::Mode::Plan, &target))
        }
        Some(Command::Download { target }) => {
            runtime.block_on(headless::run(&cli, headless::Mode::Download, &target))
        }
        Some(Command::Tui { .. }) | None => {
            eprintln!("the terminal UI is not available yet; use `gallica download`");
            EXIT_USAGE
        }
    };
    ExitCode::from(code)
}

fn init_tracing(verbose: u8) {
    let filter = match verbose {
        0 => "warn",
        1 => "info,gallica=debug,gallica_core=debug",
        _ => "debug",
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .with_writer(std::io::stderr)
        .try_init();
}
