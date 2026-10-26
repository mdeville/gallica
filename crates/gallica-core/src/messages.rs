//! Actions (from the user), events (from the engine) and commands (to the
//! engine).

use std::path::PathBuf;
use std::time::Duration;

use gallica::QuotaClass;

use crate::job::JobFile;
use crate::log::LogLevel;
use crate::settings::{Settings, VariantSpec};
use crate::state::{ItemId, Plan, Resolved};

/// What the user can do. Identical for every front end.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// The target text changed.
    SetInput(String),
    /// Resolve the target text.
    Resolve,
    /// Restrict a periodical to these years (inclusive).
    SetYears(Option<(u16, u16)>),
    /// Keep only the first N items of the plan (test runs).
    SetMaxIssues(Option<usize>),
    /// How images are fetched.
    SetVariant(VariantSpec),
    /// Where files go.
    SetOutputDir(PathBuf),
    /// Overwrite existing files.
    SetOverwrite(bool),
    /// Also fetch the plain text of each item.
    SetSaveText(bool),
    /// Contact address for the User-Agent.
    SetContact(String),
    /// Build (or rebuild) the plan.
    BuildPlan,
    /// Start the run.
    Start,
    /// Pause the run.
    Pause,
    /// Resume the run.
    Resume,
    /// Cancel the run.
    Cancel,
    /// Run again the items that failed.
    RetryFailed,
    /// Load a previous job file.
    OpenJob(PathBuf),
    /// Leave the application.
    Quit,
}

/// What the engine reports.
#[derive(Debug, Clone)]
pub enum Event {
    /// The target was resolved, or not.
    Resolved(Result<Resolved, String>),
    /// The plan was built, or not.
    Planned(Result<Plan, String>),
    /// Work on an item started.
    ItemStarted(ItemId),
    /// The number of views of an item is known.
    ItemViews {
        /// Item.
        item: ItemId,
        /// Number of views.
        total: u32,
    },
    /// One view was fetched or skipped.
    ViewDone {
        /// Item.
        item: ItemId,
        /// View number.
        view: u32,
        /// Bytes written (0 when skipped).
        bytes: u64,
        /// `true` when the file already existed.
        skipped: bool,
    },
    /// One view failed for good.
    ViewFailed {
        /// Item.
        item: ItemId,
        /// View number.
        view: u32,
        /// Why.
        error: String,
    },
    /// Work on an item finished.
    ItemFinished {
        /// Item.
        item: ItemId,
        /// Outcome.
        result: Result<ItemOutcome, String>,
    },
    /// A request is waiting for a quota.
    QuotaWait {
        /// Which quota.
        class: QuotaClass,
        /// For how long.
        wait: Duration,
    },
    /// A log line.
    Log(LogLevel, String),
    /// Gallica refuses our requests; the run stops.
    Blocked(String),
    /// The run is over.
    RunFinished {
        /// Items that failed.
        failed: u32,
        /// `true` when stopped by Cancel.
        cancelled: bool,
    },
    /// A job file was loaded, or not.
    JobLoaded(Result<Box<JobFile>, String>),
}

/// How an item ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemOutcome {
    /// Fetched in this run.
    Done,
    /// Already complete from a previous run.
    Skipped {
        /// Views recorded by the previous run.
        views: u32,
    },
}

/// What the engine is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Resolve a target.
    Resolve {
        /// URL or ARK.
        input: String,
    },
    /// Build the plan.
    BuildPlan {
        /// The resolved target.
        resolved: Resolved,
        /// Settings (years, max issues, variant...).
        settings: Settings,
    },
    /// Start a run.
    StartRun {
        /// The resolved target.
        resolved: Resolved,
        /// The plan.
        plan: Plan,
        /// Settings.
        settings: Settings,
        /// Restrict to these items (retry), else all.
        only: Option<Vec<ItemId>>,
    },
    /// Pause the run.
    Pause,
    /// Resume the run.
    Resume,
    /// Cancel the run.
    Cancel,
    /// Read a job file.
    OpenJob(PathBuf),
    /// Save settings to the config file.
    SaveSettings(Settings),
    /// The front end should exit.
    Quit,
}
