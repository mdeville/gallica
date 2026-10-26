//! The application state: everything a front end needs to draw.

use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime};

use gallica::document::Date;
use gallica::{Ark, QuotaClass};
use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// Index of an item in the plan.
pub type ItemId = usize;

/// Maximum number of log lines kept.
pub const LOG_CAPACITY: usize = 500;

/// The whole state.
#[derive(Debug, Clone, Default)]
pub struct AppState {
    /// What the user asked for and what it resolved to.
    pub target: Target,
    /// Current settings.
    pub settings: Settings,
    /// What a run would download, once planned.
    pub plan: Option<Plan>,
    /// The current or last run.
    pub run: Run,
    /// Recent log lines, oldest first.
    pub log: VecDeque<LogEntry>,
    /// Set once the user asked to quit.
    pub quit: bool,
}

/// The target input and its resolution.
#[derive(Debug, Clone, Default)]
pub struct Target {
    /// Text typed by the user (URL or ARK).
    pub input: String,
    /// Resolution status.
    pub status: TargetStatus,
    /// What the input resolved to.
    pub resolved: Option<Resolved>,
}

/// Status of the target resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TargetStatus {
    /// Nothing resolved yet.
    #[default]
    Idle,
    /// Resolution in progress.
    Resolving,
    /// Resolved.
    Resolved,
    /// Resolution failed with this message.
    Failed(String),
}

/// What a target resolved to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Resolved {
    /// A periodical title.
    Periodical {
        /// The periodical ARK.
        ark: Ark,
        /// Title, when it could be found.
        title: Option<String>,
        /// Years with issues online.
        years: Vec<u16>,
        /// Total number of issues online.
        total_issues: Option<u32>,
    },
    /// A single document.
    Document {
        /// The document ARK.
        ark: Ark,
        /// Title.
        title: Option<String>,
        /// Date as given by the record.
        date: Option<String>,
        /// Document type (`fascicule`, `monographie`...).
        typedoc: Option<String>,
        /// Number of views.
        views: u32,
        /// Rights statements.
        rights: Vec<String>,
    },
}

impl Resolved {
    /// The ARK.
    pub fn ark(&self) -> &Ark {
        match self {
            Resolved::Periodical { ark, .. } | Resolved::Document { ark, .. } => ark,
        }
    }

    /// The title, if known.
    pub fn title(&self) -> Option<&str> {
        match self {
            Resolved::Periodical { title, .. } | Resolved::Document { title, .. } => {
                title.as_deref()
            }
        }
    }

    /// `true` for a periodical.
    pub fn is_periodical(&self) -> bool {
        matches!(self, Resolved::Periodical { .. })
    }

    /// One line description.
    pub fn summary(&self) -> String {
        match self {
            Resolved::Periodical {
                years,
                total_issues,
                ..
            } => {
                let span = match (years.first(), years.last()) {
                    (Some(a), Some(b)) if a != b => format!("{a}–{b}"),
                    (Some(a), _) => a.to_string(),
                    _ => "no year".into(),
                };
                match total_issues {
                    Some(n) => format!("Periodical, {span}, {n} issues"),
                    None => format!("Periodical, {span}"),
                }
            }
            Resolved::Document {
                date,
                typedoc,
                views,
                ..
            } => {
                let mut s = typedoc.clone().unwrap_or_else(|| "Document".into());
                if let Some(d) = date {
                    s.push_str(&format!(", {d}"));
                }
                s.push_str(&format!(", {views} views"));
                s
            }
        }
    }
}

/// What a run would download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// One item per issue (periodical) or one item (document).
    pub items: Vec<PlanItem>,
    /// Cost estimate.
    pub estimate: Estimate,
}

/// One unit of work: a document to download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    /// Position in the plan.
    pub id: ItemId,
    /// The document ARK.
    pub ark: Ark,
    /// Publication date, when known.
    pub date: Option<Date>,
    /// Publication year, when known.
    pub year: Option<u16>,
    /// Label as given by Gallica (`03 novembre 1921`, a title...).
    pub label: String,
    /// Number of views, when already known.
    pub views: Option<u32>,
}

/// Cost estimate of a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    /// Number of items.
    pub items: usize,
    /// Number of views (images).
    pub views: u64,
    /// `false` when `views` is a guess from an average.
    pub views_known: bool,
    /// Metadata requests (records, page lists).
    pub metadata_requests: u64,
    /// Image requests.
    pub image_requests: u64,
    /// Quota class the images fall in.
    pub image_class: QuotaClass,
    /// Text requests, when `save_text` is on.
    pub text_requests: u64,
    /// Least duration under the quotas, `None` when unlimited.
    pub minimum: Option<Duration>,
    /// Rough size of the download.
    pub bytes_guess: u64,
}

/// The current or last run.
#[derive(Debug, Clone, Default)]
pub struct Run {
    /// Where we are.
    pub phase: Phase,
    /// One entry per plan item.
    pub items: Vec<ItemState>,
    /// Counters.
    pub progress: Progress,
    /// Set when the user asked to cancel the current run.
    pub cancel_requested: bool,
}

/// Phase of the application.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Phase {
    /// Nothing to do.
    #[default]
    Idle,
    /// Resolving the target.
    Resolving,
    /// Building the plan.
    Planning,
    /// Plan ready, waiting for Start.
    Ready,
    /// Downloading.
    Running,
    /// Paused by the user.
    Paused,
    /// Run finished (possibly with failed items).
    Finished,
    /// Run aborted with this message.
    Failed(String),
}

impl Phase {
    /// `true` while a run is in progress (running or paused).
    pub fn is_active(&self) -> bool {
        matches!(self, Phase::Running | Phase::Paused)
    }

    /// Short label.
    pub fn label(&self) -> &str {
        match self {
            Phase::Idle => "Idle",
            Phase::Resolving => "Resolving",
            Phase::Planning => "Planning",
            Phase::Ready => "Ready",
            Phase::Running => "Running",
            Phase::Paused => "Paused",
            Phase::Finished => "Finished",
            Phase::Failed(_) => "Failed",
        }
    }
}

/// State of one plan item during a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemState {
    /// Status.
    pub status: ItemStatus,
    /// Views fetched (or skipped) so far.
    pub done_views: u32,
    /// Total views, once known.
    pub total_views: Option<u32>,
    /// Bytes written for this item.
    pub bytes: u64,
    /// Error message, when failed.
    pub error: Option<String>,
}

/// Status of one item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ItemStatus {
    /// Not started.
    #[default]
    Pending,
    /// In progress.
    Fetching,
    /// Completed in this run.
    Done,
    /// Already complete from a previous run.
    Skipped,
    /// Failed.
    Failed,
}

impl ItemStatus {
    /// Short label.
    pub fn label(&self) -> &'static str {
        match self {
            ItemStatus::Pending => "pending",
            ItemStatus::Fetching => "fetching",
            ItemStatus::Done => "done",
            ItemStatus::Skipped => "skipped",
            ItemStatus::Failed => "failed",
        }
    }
}

/// Run counters.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    /// Items finished (done, skipped or failed).
    pub items_done: usize,
    /// Items in the run.
    pub items_total: usize,
    /// Views fetched or skipped.
    pub views_done: u64,
    /// Total views, refined as page lists arrive.
    pub views_total: Option<u64>,
    /// Bytes written.
    pub bytes: u64,
    /// When the run started.
    pub started_at: Option<Instant>,
    /// When the run finished.
    pub finished_at: Option<Instant>,
    /// Number of quota waits so far.
    pub quota_waits: u32,
    /// Current quota wait, if any: class, duration, when it started.
    pub quota_waiting: Option<(QuotaClass, Duration, Instant)>,
    /// Items that failed.
    pub failed_items: u32,
}

impl Progress {
    /// Elapsed time of the run.
    pub fn elapsed(&self) -> Duration {
        match (self.started_at, self.finished_at) {
            (Some(s), Some(f)) => f.duration_since(s),
            (Some(s), None) => s.elapsed(),
            _ => Duration::ZERO,
        }
    }

    /// Fraction done in `0.0..=1.0`, by views when known, else by items.
    pub fn fraction(&self) -> f64 {
        match self.views_total {
            Some(total) if total > 0 => (self.views_done as f64 / total as f64).min(1.0),
            _ if self.items_total > 0 => self.items_done as f64 / self.items_total as f64,
            _ => 0.0,
        }
    }

    /// Average download rate in bytes per second.
    pub fn rate(&self) -> f64 {
        let secs = self.elapsed().as_secs_f64();
        if secs <= 0.0 {
            0.0
        } else {
            self.bytes as f64 / secs
        }
    }

    /// Estimated remaining time, from the observed pace.
    pub fn eta(&self) -> Option<Duration> {
        let total = self.views_total?;
        if self.views_done == 0 || self.finished_at.is_some() {
            return None;
        }
        let remaining = total.saturating_sub(self.views_done);
        let per_view = self.elapsed().as_secs_f64() / self.views_done as f64;
        Some(Duration::from_secs_f64(per_view * remaining as f64))
    }

    /// The quota wait still in progress, if any.
    pub fn current_quota_wait(&self) -> Option<(QuotaClass, Duration)> {
        let (class, wait, since) = self.quota_waiting?;
        let remaining = wait.checked_sub(since.elapsed())?;
        Some((class, remaining))
    }
}

/// A log line.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// When.
    pub at: SystemTime,
    /// Level.
    pub level: LogLevel,
    /// Text.
    pub message: String,
}

/// Log level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Information.
    Info,
    /// Something to look at.
    Warn,
    /// Something failed.
    Error,
}

impl LogLevel {
    /// Short label.
    pub fn label(&self) -> &'static str {
        match self {
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}
