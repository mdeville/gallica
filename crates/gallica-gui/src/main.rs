//! The `gallica-gui` binary.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod panels;
mod state;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use gallica_core::engine::channel;
use gallica_core::{App, Engine, Settings};

/// Bulk downloader for Gallica, the digital library of the BnF.
#[derive(Debug, Parser)]
#[command(name = "gallica-gui", version, about)]
struct Cli {
    /// Settings file (default: the platform configuration directory).
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Gallica URL or ARK to start with.
    target: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .try_init();

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
                cc, core, engine, rx, handle, target,
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
