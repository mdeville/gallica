//! The application core of the Gallica bulk downloader, shared by the
//! terminal UI, the desktop GUI and the headless commands.
//!
//! The core follows an Elm-like shape:
//!
//! - [`AppState`] is everything a front end needs to draw;
//! - [`Action`] is what the user can do, [`Event`] what the engine reports;
//! - [`App::update`] and [`App::apply`] are pure state transitions returning
//!   [`Command`]s;
//! - [`Engine`] executes commands against the Gallica SDK on a tokio runtime
//!   and sends events back through a channel.
//!
//! See `docs/SPEC.md` at the repository root.

pub mod app;
pub mod engine;
pub mod format;
pub mod job;
pub mod layout;
pub mod messages;
pub mod plan;
pub mod settings;
pub mod state;

pub use app::App;
pub use engine::Engine;
pub use messages::{Action, Command, Event, ItemOutcome};
pub use settings::{Settings, VariantSpec};
pub use state::{
    AppState, Estimate, ItemId, ItemState, ItemStatus, LogEntry, LogLevel, Phase, Plan, PlanItem,
    Progress, Resolved, Run, Target, TargetStatus,
};

/// Re-export of the SDK for front ends that need its types.
pub use gallica;
