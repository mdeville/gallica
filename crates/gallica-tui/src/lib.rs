//! The `gallica` binary: headless commands (`info`, `plan`, `download`) and
//! the terminal UI (`tui`).

pub mod cli;
pub mod headless;
pub mod tui;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;
use gallica_core::LogBuffer;
use gallica_core::log::{self, LogLayer};

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
    let headless = matches!(
        cli.command,
        Some(Command::Info { .. } | Command::Plan { .. } | Command::Download { .. })
    );
    let printer = headless::Printer::default();
    let buffer = LogBuffer::new();
    if headless {
        log::init(cli.verbosity(), vec![printer.layer().boxed()]);
    } else {
        let mut outputs = vec![buffer.layer().boxed()];
        let path = log::create_file("gallica").map(|(path, file)| {
            outputs.push(LogLayer::file(file).boxed());
            path
        });
        log::init(cli.verbosity(), outputs);
        if let Some(path) = path {
            tracing::debug!("log file: {}", path.display());
        }
    }
    let code = match cli.command.clone() {
        Some(Command::Info { target }) => {
            runtime.block_on(headless::run(&cli, &printer, headless::Mode::Info, &target))
        }
        Some(Command::Plan { target }) => {
            runtime.block_on(headless::run(&cli, &printer, headless::Mode::Plan, &target))
        }
        Some(Command::Download { target }) => runtime.block_on(headless::run(
            &cli,
            &printer,
            headless::Mode::Download,
            &target,
        )),
        Some(Command::Tui { target }) => runtime.block_on(tui::run(&cli, buffer, target)),
        None => runtime.block_on(tui::run(&cli, buffer, None)),
    };
    ExitCode::from(code)
}
