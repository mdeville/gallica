//! Logging: everything goes through `tracing`.
//!
//! Messages meant for the user (resolution, plan, failures, run summary) are
//! events with the [`TARGET`] target, emitted by [`App::log`]; the SDK and
//! the engine keep their module targets for diagnostics. One filter, chosen
//! by [`Verbosity`] (or `RUST_LOG`), applies to every output, and every
//! output formats a [`LogEntry`] the same way, so the headless commands, the
//! terminal UI's log panel, the GUI's log panel and the log file show the
//! same stream.
//!
//! [`App::log`]: crate::App::log

use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::fs::File;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_log::NormalizeEvent;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime as Rfc3339};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry};

/// Target of the application messages.
pub const TARGET: &str = "gallica_app";

/// Maximum number of entries a [`LogBuffer`] keeps.
pub const LOG_CAPACITY: usize = 500;

/// How much to log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Verbosity {
    /// Application messages, and warnings and errors from everything else.
    #[default]
    Normal,
    /// Also informational diagnostics, and debug ones from this project.
    Verbose,
    /// Every diagnostic, dependencies included.
    Debug,
}

impl Verbosity {
    /// The `tracing` filter directives for this verbosity.
    pub fn directives(self) -> String {
        match self {
            Verbosity::Normal => format!("warn,{TARGET}=info"),
            Verbosity::Verbose => "info,gallica=debug,gallica_core=debug".into(),
            Verbosity::Debug => "debug".into(),
        }
    }

    /// The filter: `RUST_LOG` when set, else this verbosity.
    pub fn filter(self) -> EnvFilter {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(self.directives()))
    }
}

/// A boxed output for [`init`].
pub type BoxLayer = Box<dyn Layer<Registry> + Send + Sync>;

/// Installs the global subscriber: the verbosity's filter in front of
/// `outputs`. Does nothing if a subscriber is already installed.
pub fn init(verbosity: Verbosity, outputs: Vec<BoxLayer>) {
    let _ = tracing_subscriber::registry()
        .with(outputs)
        .with(verbosity.filter())
        .try_init();
}

/// A log line.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// When.
    pub at: SystemTime,
    /// Level.
    pub level: LogLevel,
    /// Where a diagnostic comes from; `None` for application messages.
    pub target: Option<String>,
    /// Text, with the event's fields appended as `key=value`.
    pub message: String,
}

impl LogEntry {
    /// An application message.
    pub fn new(level: LogLevel, message: impl Into<String>) -> LogEntry {
        LogEntry {
            at: SystemTime::now(),
            level,
            target: None,
            message: message.into(),
        }
    }

    /// The text after the level: the message, prefixed by the target for
    /// diagnostics.
    pub fn text(&self) -> String {
        match &self.target {
            Some(target) => format!("{target}: {}", self.message),
            None => self.message.clone(),
        }
    }

    /// The line as printed: level, then [`text`](Self::text).
    pub fn line(&self) -> String {
        format!("{:<5} {}", self.level.label(), self.text())
    }
}

/// Log level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Fine-grained diagnostics.
    Trace,
    /// Diagnostics.
    Debug,
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
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

impl From<Level> for LogLevel {
    fn from(level: Level) -> LogLevel {
        match level {
            Level::TRACE => LogLevel::Trace,
            Level::DEBUG => LogLevel::Debug,
            Level::INFO => LogLevel::Info,
            Level::WARN => LogLevel::Warn,
            Level::ERROR => LogLevel::Error,
        }
    }
}

/// Emits an application message.
pub fn emit(level: LogLevel, message: &str) {
    match level {
        LogLevel::Trace => tracing::trace!(target: TARGET, "{message}"),
        LogLevel::Debug => tracing::debug!(target: TARGET, "{message}"),
        LogLevel::Info => tracing::info!(target: TARGET, "{message}"),
        LogLevel::Warn => tracing::warn!(target: TARGET, "{message}"),
        LogLevel::Error => tracing::error!(target: TARGET, "{message}"),
    }
}

/// An output: turns each event into a [`LogEntry`] and hands it to a sink.
pub struct LogLayer {
    sink: Box<dyn Fn(LogEntry) + Send + Sync>,
}

impl LogLayer {
    /// An output calling `sink` for each event.
    pub fn new(sink: impl Fn(LogEntry) + Send + Sync + 'static) -> LogLayer {
        LogLayer {
            sink: Box::new(sink),
        }
    }

    /// An output printing lines to stderr.
    pub fn stderr() -> LogLayer {
        LogLayer::new(|entry| eprintln!("{}", entry.line()))
    }

    /// An output appending timestamped lines to `file`.
    pub fn file(file: File) -> LogLayer {
        let file = Mutex::new(file);
        LogLayer::new(move |entry| {
            let mut stamp = String::new();
            let _ = Rfc3339.format_time(&mut Writer::new(&mut stamp));
            if let Ok(mut f) = file.lock() {
                let _ = writeln!(f, "{stamp} {}", entry.line());
            }
        })
    }

    /// Boxes the output for [`init`].
    pub fn boxed(self) -> BoxLayer {
        Box::new(self)
    }
}

impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // Records from the `log` crate (reqwest, eframe...) carry their
        // real target in fields.
        let normalized = event.normalized_metadata();
        let meta = normalized.as_ref().unwrap_or_else(|| event.metadata());
        let mut visitor = Fields::default();
        event.record(&mut visitor);
        let target = meta.target();
        (self.sink)(LogEntry {
            at: SystemTime::now(),
            level: (*meta.level()).into(),
            target: (target != TARGET).then(|| target.to_string()),
            message: visitor.finish(),
        });
    }
}

/// Collects an event's message and fields.
#[derive(Default)]
struct Fields {
    message: String,
    fields: String,
}

impl Fields {
    fn finish(self) -> String {
        if self.message.is_empty() {
            self.fields
        } else if self.fields.is_empty() {
            self.message
        } else {
            format!("{} {}", self.message, self.fields)
        }
    }

    fn push(&mut self, field: &Field, value: fmt::Arguments<'_>) {
        if field.name() == "message" {
            let _ = self.message.write_fmt(value);
        } else if !field.name().starts_with("log.") {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}={value}", field.name());
        }
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field, format_args!("{value}"));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.push(field, format_args!("{value:?}"));
    }
}

/// The most recent entries, shared between an output and a front end that
/// shows them.
#[derive(Debug, Clone, Default)]
pub struct LogBuffer {
    entries: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl LogBuffer {
    /// An empty buffer.
    pub fn new() -> LogBuffer {
        LogBuffer::default()
    }

    /// An output appending to this buffer.
    pub fn layer(&self) -> LogLayer {
        let buffer = self.clone();
        LogLayer::new(move |entry| buffer.push(entry))
    }

    /// Appends an entry, dropping the oldest beyond [`LOG_CAPACITY`].
    pub fn push(&self, entry: LogEntry) {
        if let Ok(mut entries) = self.entries.lock() {
            if entries.len() >= LOG_CAPACITY {
                entries.pop_front();
            }
            entries.push_back(entry);
        }
    }

    /// A copy of the entries, oldest first. Copying keeps the lock out of
    /// drawing code, which may itself log.
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.entries
            .lock()
            .map(|e| e.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Removes every entry.
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }
}

/// Where a front end named `name` writes its log file: the platform state
/// (or local data) directory.
pub fn file_path(name: &str) -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "gallica")?;
    let dir = dirs.state_dir().unwrap_or_else(|| dirs.data_local_dir());
    Some(dir.join(format!("{name}.log")))
}

/// Creates (truncating) the log file of the front end named `name`.
pub fn create_file(name: &str) -> Option<(PathBuf, File)> {
    let path = file_path(name)?;
    std::fs::create_dir_all(path.parent()?).ok()?;
    let file = File::create(&path).ok()?;
    Some((path, file))
}

/// Captures this thread's events into a buffer until the guard drops; for
/// tests. The first call installs a global subscriber that hands each event
/// to its thread's capture, if any: per-thread subscribers would race over
/// the callsites' cached interest when tests run in parallel and lose
/// events. Do not mix with [`init`].
pub fn capture() -> (LogBuffer, CaptureGuard) {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let layer = LogLayer::new(|entry| {
            CAPTURE.with(|c| {
                if let Some(buffer) = &*c.borrow() {
                    buffer.push(entry);
                }
            })
        });
        let _ = tracing_subscriber::registry().with(layer).try_init();
    });
    let buffer = LogBuffer::new();
    CAPTURE.with(|c| *c.borrow_mut() = Some(buffer.clone()));
    (buffer, CaptureGuard(()))
}

thread_local! {
    static CAPTURE: std::cell::RefCell<Option<LogBuffer>> = const { std::cell::RefCell::new(None) };
}

/// Stops a [`capture`] when dropped.
pub struct CaptureGuard(());

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CAPTURE.with(|c| *c.borrow_mut() = None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_messages_and_diagnostics_alike() {
        let (buffer, _guard) = capture();
        emit(LogLevel::Warn, "Paused");
        tracing::debug!(target: "gallica::client", url = %"https://x", attempt = 2, "GET");
        tracing_log::format_trace(
            &::log::Record::builder()
                .level(::log::Level::Debug)
                .target("reqwest::connect")
                .args(format_args!("starting new connection"))
                .build(),
        )
        .unwrap();
        let lines: Vec<_> = buffer.snapshot().iter().map(LogEntry::line).collect();
        assert_eq!(
            lines,
            [
                "warn  Paused",
                "debug gallica::client: GET url=https://x attempt=2",
                "debug reqwest::connect: starting new connection",
            ]
        );
    }

    #[test]
    fn buffer_keeps_the_most_recent() {
        let buffer = LogBuffer::new();
        for i in 0..LOG_CAPACITY + 3 {
            buffer.push(LogEntry::new(LogLevel::Info, i.to_string()));
        }
        let entries = buffer.snapshot();
        assert_eq!(entries.len(), LOG_CAPACITY);
        assert_eq!(entries[0].message, "3");
    }
}
