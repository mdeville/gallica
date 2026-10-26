//! The terminal UI (ratatui).

pub mod input;
pub mod state;
pub mod ui;

use std::io::IsTerminal;
use std::time::Duration;

use crossterm::event::{Event as TermEvent, EventStream, KeyEventKind};
use futures::StreamExt;
use gallica_core::engine::channel;
use gallica_core::{Action, App, Engine};

use crate::EXIT_USAGE;
use crate::cli::{Cli, load_settings};
use state::UiState;

/// Runs the terminal UI and returns the exit code.
pub async fn run(cli: &Cli, target: Option<String>) -> u8 {
    if !std::io::stdout().is_terminal() {
        eprintln!("the terminal UI needs a terminal; use `gallica download` for scripts");
        return EXIT_USAGE;
    }
    let settings = match load_settings(cli) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_USAGE;
        }
    };
    let (tx, mut rx) = channel();
    let mut engine = match Engine::new(&settings, tx, tokio::runtime::Handle::current()) {
        Ok(e) => e.with_config_path(cli.config.clone()),
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_USAGE;
        }
    };
    let mut app = App::new(settings);
    let mut ui = UiState::default();
    if let Some(t) = target {
        for cmd in app.update(Action::SetInput(t)) {
            engine.execute(cmd);
        }
        for cmd in app.update(Action::Resolve) {
            engine.execute(cmd);
        }
    } else {
        ui.edit = Some(state::Edit::new(state::Field::Target, String::new()));
    }

    let mut terminal = ratatui::init();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let result: std::io::Result<()> = loop {
        if let Err(e) = terminal.draw(|frame| ui::draw(frame, &mut ui, &app.state)) {
            break Err(e);
        }
        tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(TermEvent::Key(key))) if key.kind == KeyEventKind::Press => {
                    for action in input::handle_key(key, &mut ui, &app.state) {
                        for cmd in app.update(action) {
                            engine.execute(cmd);
                        }
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => break Err(e),
                None => break Ok(()),
            },
            ev = rx.recv() => match ev {
                Some(event) => {
                    for cmd in app.apply(event) {
                        engine.execute(cmd);
                    }
                }
                None => break Ok(()),
            },
            _ = tick.tick() => {}
        }
        if app.state.quit {
            break Ok(());
        }
    };
    ratatui::restore();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("terminal error: {e}");
            EXIT_USAGE
        }
    }
}
