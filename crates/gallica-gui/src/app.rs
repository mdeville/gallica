//! The eframe application: pumps engine events and draws the panels.

use std::sync::mpsc;

use gallica_core::engine::EventReceiver;
use gallica_core::{Action, App, Engine, Event, LogLevel};

use crate::panels;
use crate::state::{Confirm, GuiState};

/// The application.
pub struct GuiApp {
    /// The core.
    pub core: App,
    /// The engine.
    pub engine: Engine,
    /// GUI-only state.
    pub gui: GuiState,
    /// Events forwarded from the engine.
    events: mpsc::Receiver<Event>,
    /// Runtime handle for background tasks (file dialogs).
    pub handle: tokio::runtime::Handle,
}

impl GuiApp {
    /// Creates the application and starts forwarding engine events to the
    /// GUI thread, repainting on each.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        core: App,
        engine: Engine,
        mut rx: EventReceiver,
        handle: tokio::runtime::Handle,
        target: Option<String>,
    ) -> GuiApp {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let (tx, events) = mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        handle.spawn(async move {
            while let Some(event) = rx.recv().await {
                if tx.send(event).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        let mut app = GuiApp {
            core,
            engine,
            gui: GuiState {
                show_log: true,
                ..Default::default()
            },
            events,
            handle,
        };
        app.gui.buffers.sync_from(&app.core.state);
        app.gui.synced = true;
        if let Some(t) = target {
            app.dispatch(Action::SetInput(t));
            app.dispatch(Action::Resolve);
            app.gui.buffers.target = app.core.state.target.input.clone();
        }
        app
    }

    /// Applies an action and executes the resulting commands.
    pub fn dispatch(&mut self, action: Action) {
        for cmd in self.core.update(action) {
            self.engine.execute(cmd);
        }
    }

    fn pump_events(&mut self) {
        let mut resync = false;
        while let Ok(event) = self.events.try_recv() {
            if matches!(event, Event::JobLoaded(Ok(_))) {
                resync = true;
            }
            for cmd in self.core.apply(event) {
                self.engine.execute(cmd);
            }
        }
        if resync {
            self.gui.buffers.sync_from(&self.core.state);
        }
        if let Some(rx) = &self.gui.folder_picker {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    self.gui.folder_picker = None;
                    self.gui.buffers.output_dir = path.display().to_string();
                    self.dispatch(Action::SetOutputDir(path));
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => self.gui.folder_picker = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.gui.job_picker {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    self.gui.job_picker = None;
                    self.dispatch(Action::OpenJob(path));
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => self.gui.job_picker = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    /// Asks before quitting if a run is active, else quits.
    pub fn request_quit(&mut self) {
        if self.core.state.run.phase.is_active() {
            self.gui.confirm = Some(Confirm::QuitRunning);
        } else {
            self.dispatch(Action::Quit);
        }
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_events();

        if ctx.input(|i| i.viewport().close_requested()) && !self.core.state.quit {
            if self.core.state.run.phase.is_active()
                && self.gui.confirm != Some(Confirm::QuitRunning)
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.gui.confirm = Some(Confirm::QuitRunning);
            } else {
                self.dispatch(Action::Quit);
            }
        }

        panels::menu_bar(self, ctx);
        panels::target_panel(self, ctx);
        panels::side_panel(self, ctx);
        panels::bottom_panel(self, ctx);
        panels::queue_panel(self, ctx);
        panels::dialogs(self, ctx);

        if self.core.state.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Keep the progress line and ETA fresh during a run.
        if self.core.state.run.phase.is_active() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

/// Convenience for panels: log a line without going through the engine.
pub fn warn(app: &mut GuiApp, message: impl Into<String>) {
    app.core.log(LogLevel::Warn, message);
}
