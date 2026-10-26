//! Files written next to the downloads: job description, per-item metadata
//! and completion markers.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use gallica::Ark;
use gallica::document::{OaiRecord, Pagination};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::settings::{Settings, VariantSpec};
use crate::state::{Plan, Resolved};

/// Format version of the job file.
pub const JOB_VERSION: u32 = 1;

/// Description of the last run of a target, saved at the root of its folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobFile {
    /// Format version.
    pub version: u32,
    /// The text the user entered.
    pub input: String,
    /// What it resolved to.
    pub resolved: Resolved,
    /// Settings of the run.
    pub settings: Settings,
    /// The plan.
    pub plan: Plan,
    /// Unix time of the save.
    pub saved_at: u64,
}

/// Per-item bibliographic record and page list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemMetadata {
    /// The document ARK.
    pub ark: Ark,
    /// The bibliographic record.
    pub record: OaiRecord,
    /// The page list.
    pub pagination: Pagination,
}

/// Per-item completion marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoneMarker {
    /// The document ARK.
    pub ark: Ark,
    /// Variant the images were fetched with.
    pub variant: VariantSpec,
    /// Number of views.
    pub views: u32,
    /// Whether the text was saved too.
    pub text: bool,
    /// Unix time of completion.
    pub finished_at: u64,
}

/// Per-periodical description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeriodicalFile {
    /// The periodical ARK.
    pub ark: Ark,
    /// Title.
    pub title: Option<String>,
    /// Years with issues online.
    pub years: Vec<u16>,
    /// Total number of issues online.
    pub total_issues: Option<u32>,
}

/// Seconds since the Unix epoch.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Writes a value as pretty JSON, atomically.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.part");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Reads a JSON file.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> std::io::Result<T> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(std::io::Error::other)
}
