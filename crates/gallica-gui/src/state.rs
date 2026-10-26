//! GUI-only state: text buffers, selection, dialogs.

use std::path::PathBuf;
use std::sync::mpsc;

use gallica_core::{AppState, VariantSpec};

/// A yes/no question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    /// Start a full resolution run.
    StartFullRes,
    /// Quit while a run is active.
    QuitRunning,
    /// Cancel the run.
    Cancel,
}

impl Confirm {
    /// The question.
    pub fn message(self) -> &'static str {
        match self {
            Confirm::StartFullRes => {
                "Full resolution is limited to 5 images per minute by the BnF. The estimate in Details applies. Start anyway?"
            }
            Confirm::QuitRunning => "A run is in progress. Quit and cancel it?",
            Confirm::Cancel => "Cancel the run after the current views?",
        }
    }
}

/// Text buffers mirrored from the settings while they are edited.
#[derive(Debug, Default)]
pub struct Buffers {
    /// Target URL or ARK.
    pub target: String,
    /// Years, as `1921-1925` or empty.
    pub years: String,
    /// Max issues, as a number or empty.
    pub max_issues: String,
    /// Output directory.
    pub output_dir: String,
    /// Contact address.
    pub contact: String,
    /// Width for the `width:<px>` variant.
    pub width: String,
    /// Which fixed variant choice is selected.
    pub variant: VariantSpec,
}

impl Buffers {
    /// Refreshes every buffer from the state (after settings changed outside
    /// the text fields, e.g. a loaded job).
    pub fn sync_from(&mut self, app: &AppState) {
        self.target = app.target.input.clone();
        self.years = app
            .settings
            .years
            .map(|(a, b)| {
                if a == b {
                    a.to_string()
                } else {
                    format!("{a}-{b}")
                }
            })
            .unwrap_or_default();
        self.max_issues = app
            .settings
            .max_issues
            .map(|n| n.to_string())
            .unwrap_or_default();
        self.output_dir = app.settings.output_dir.display().to_string();
        self.contact = app.settings.contact.clone().unwrap_or_default();
        self.variant = app.settings.variant;
        if let VariantSpec::Width(w) = app.settings.variant {
            self.width = w.to_string();
        } else if self.width.is_empty() {
            self.width = "1000".into();
        }
    }
}

/// All GUI-only state.
#[derive(Default)]
pub struct GuiState {
    /// Text buffers.
    pub buffers: Buffers,
    /// Selected queue row.
    pub selected: Option<usize>,
    /// Pending question.
    pub confirm: Option<Confirm>,
    /// About window shown.
    pub show_about: bool,
    /// Log panel expanded.
    pub show_log: bool,
    /// Result channel of a folder picker in progress.
    pub folder_picker: Option<mpsc::Receiver<Option<PathBuf>>>,
    /// Result channel of a job file picker in progress.
    pub job_picker: Option<mpsc::Receiver<Option<PathBuf>>>,
    /// Set once buffers have been initialised from the state.
    pub synced: bool,
}

/// Parses `1921-1925` or `1921`.
pub fn parse_years(s: &str) -> Result<Option<(u16, u16)>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let (a, b) = match s.split_once('-') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (s, s),
    };
    let a: u16 = a.parse().map_err(|_| format!("invalid year {a:?}"))?;
    let b: u16 = b.parse().map_err(|_| format!("invalid year {b:?}"))?;
    Ok(Some(if a <= b { (a, b) } else { (b, a) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn years() {
        assert_eq!(parse_years("").unwrap(), None);
        assert_eq!(parse_years("1925-1921").unwrap(), Some((1921, 1925)));
        assert_eq!(parse_years(" 1921 ").unwrap(), Some((1921, 1921)));
        assert!(parse_years("x").is_err());
    }
}
