//! The `gallica-gui` binary.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod panels;
mod state;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use gallica_core::engine::channel;
use gallica_core::log::{self, LogLayer};
use gallica_core::{App, Engine, LogBuffer, Settings, Verbosity};

/// Bulk downloader for Gallica, the digital library of the BnF.
#[derive(Debug, Parser)]
#[command(name = "gallica-gui", version, about)]
struct Cli {
    /// Settings file (default: the platform configuration directory).
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// More diagnostics (-v: info, and debug from gallica; -vv: as --debug).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Every diagnostic, dependencies included.
    #[arg(long)]
    debug: bool,
    /// Gallica URL or ARK to start with.
    target: Option<String>,
}

/// The log goes to the log panel, to stderr (when launched from a
/// terminal) and to gallica-gui.log in the platform state directory.
fn init_log(cli: &Cli) -> LogBuffer {
    let verbosity = match (cli.debug, cli.verbose) {
        (true, _) | (_, 2..) => Verbosity::Debug,
        (false, 1) => Verbosity::Verbose,
        (false, 0) => Verbosity::Normal,
    };
    let buffer = LogBuffer::new();
    let mut outputs = vec![buffer.layer().boxed(), LogLayer::stderr().boxed()];
    let path = log::create_file("gallica-gui").map(|(path, file)| {
        outputs.push(LogLayer::file(file).boxed());
        path
    });
    log::init(verbosity, outputs);
    if let Some(path) = path {
        tracing::debug!("log file: {}", path.display());
    }
    buffer
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let log = init_log(&cli);

    let settings = match &cli.config {
        Some(p) => Settings::load_from(p),
        None => Settings::load(),
    };
    let settings = match settings {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("cannot start the async runtime: {e}");
            return ExitCode::from(2);
        }
    };
    let (tx, rx) = channel();
    let engine = match Engine::new(&settings, tx, runtime.handle().clone()) {
        Ok(e) => e.with_config_path(cli.config.clone()),
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let core = App::new(settings);
    let handle = runtime.handle().clone();
    let target = cli.target.clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Gallica downloader")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([800.0, 520.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "gallica-gui",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::GuiApp::new(
                cc, core, engine, rx, handle, log, target,
            )))
        }),
    );
    // Give in-flight tasks a moment, then stop the runtime.
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
