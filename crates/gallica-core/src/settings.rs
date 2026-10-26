//! Persistent settings.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use config::{Config, Environment, File, FileFormat};
use gallica::document::PreviewSize;
use gallica::{ClientBuilder, DEFAULT_USER_AGENT, ImageVariant, QuotaClass, Quotas};
use serde::{Deserialize, Serialize};

/// Prefix of the environment variables that override the file:
/// `GALLICA_CONTACT`, `GALLICA_OUTPUT_DIR`, `GALLICA_QUOTAS__TEXTS_PER_MINUTE`
/// (`__` separates nested keys).
pub const ENV_PREFIX: &str = "GALLICA";

/// Errors while loading or saving settings.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// No config directory could be determined.
    #[error("no configuration directory on this system")]
    NoConfigDir,
    /// File error.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The error.
        #[source]
        source: std::io::Error,
    },
    /// The file or the environment does not hold valid settings.
    #[error("{path}: {message}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        message: String,
    },
}

/// User settings, persisted as TOML in the platform configuration directory.
///
/// Loading layers, last wins: the defaults, the file, then the `GALLICA_*`
/// environment variables (see [`ENV_PREFIX`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Contact address appended to the User-Agent. Required before a run.
    pub contact: Option<String>,
    /// Where files go.
    pub output_dir: PathBuf,
    /// How page images are fetched.
    pub variant: VariantSpec,
    /// Overwrite existing files.
    pub overwrite: bool,
    /// Also fetch the plain text of each item.
    pub save_text: bool,
    /// Restrict a periodical to these years (inclusive).
    pub years: Option<(u16, u16)>,
    /// Keep only the first N items of the plan.
    pub max_issues: Option<usize>,
    /// Quota overrides, if the BnF grants more than the published limits.
    pub quotas: Option<Quotas>,
    /// Minimum delay between requests, in milliseconds.
    pub min_request_interval_ms: Option<u64>,
    /// Gallica base URL override (tests, mirrors).
    pub base_url: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
        Settings {
            contact: None,
            output_dir: home.unwrap_or_else(|| PathBuf::from(".")).join("Gallica"),
            variant: VariantSpec::HighRes,
            overwrite: false,
            save_text: false,
            years: None,
            max_issues: None,
            quotas: None,
            min_request_interval_ms: None,
            base_url: None,
        }
    }
}

impl Settings {
    /// The configuration file path on this platform.
    pub fn config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "gallica")
            .map(|d| d.config_dir().join("config.toml"))
    }

    /// Loads the settings from the configuration file (defaults when absent)
    /// and the environment.
    pub fn load() -> Result<Settings, SettingsError> {
        let path = Self::config_path().ok_or(SettingsError::NoConfigDir)?;
        Self::load_from(&path)
    }

    /// Loads the settings from a TOML file (defaults when absent) and the
    /// environment.
    pub fn load_from(path: &Path) -> Result<Settings, SettingsError> {
        Self::load_with(path, Environment::with_prefix(ENV_PREFIX))
    }

    fn load_with(path: &Path, env: Environment) -> Result<Settings, SettingsError> {
        let parse = |e: config::ConfigError| SettingsError::Parse {
            path: path.to_path_buf(),
            message: e.to_string(),
        };
        // Missing keys take their defaults through `#[serde(default)]`.
        Config::builder()
            .add_source(File::from(path).format(FileFormat::Toml).required(false))
            .add_source(env.prefix_separator("_").separator("__").try_parsing(true))
            .build()
            .map_err(parse)?
            .try_deserialize()
            .map_err(parse)
    }

    /// Saves the settings to the configuration file.
    pub fn save(&self) -> Result<PathBuf, SettingsError> {
        let path = Self::config_path().ok_or(SettingsError::NoConfigDir)?;
        self.save_to(&path)?;
        Ok(path)
    }

    /// Saves the settings to a file, creating parent directories.
    pub fn save_to(&self, path: &Path) -> Result<(), SettingsError> {
        let io = |source| SettingsError::Io {
            path: path.to_path_buf(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| SettingsError::Parse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        std::fs::write(path, text).map_err(io)
    }

    /// The User-Agent sent to Gallica.
    pub fn user_agent(&self) -> String {
        match self
            .contact
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            Some(contact) => format!("{DEFAULT_USER_AGENT} contact:{contact}"),
            None => DEFAULT_USER_AGENT.to_string(),
        }
    }

    /// `true` when a contact address is set.
    pub fn has_contact(&self) -> bool {
        self.contact
            .as_deref()
            .map(str::trim)
            .is_some_and(|c| !c.is_empty())
    }

    /// The quotas in force.
    pub fn quotas(&self) -> Quotas {
        self.quotas.clone().unwrap_or_default()
    }

    /// The image variant.
    pub fn image_variant(&self) -> ImageVariant {
        self.variant.to_image_variant()
    }

    /// A client builder configured from these settings.
    pub fn client_builder(&self) -> Result<ClientBuilder, gallica::Error> {
        let mut b = gallica::Client::builder()
            .user_agent(self.user_agent())
            .quotas(self.quotas());
        if let Some(ms) = self.min_request_interval_ms {
            b = b.min_request_interval(std::time::Duration::from_millis(ms));
        }
        if let Some(url) = &self.base_url {
            b = b.base_url(url)?;
        }
        Ok(b)
    }
}

/// How page images are fetched, in a form that reads and writes as text:
/// `highres`, `medres`, `lowres`, `thumbnail`, `width:<px>`, `full`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VariantSpec {
    /// Pre-computed preview fitting in 1024×1536 px (50 per minute).
    #[default]
    HighRes,
    /// Pre-computed preview fitting in 512×768 px.
    MedRes,
    /// Pre-computed preview fitting in 256×384 px.
    LowRes,
    /// Pre-computed preview fitting in 128×192 px.
    Thumbnail,
    /// IIIF image scaled to this width (above 1000 px: 5 per minute).
    Width(u32),
    /// Master resolution through IIIF (5 per minute).
    Full,
}

impl VariantSpec {
    /// All the fixed choices, for menus (width is free-form).
    pub const CHOICES: [VariantSpec; 6] = [
        VariantSpec::HighRes,
        VariantSpec::MedRes,
        VariantSpec::LowRes,
        VariantSpec::Thumbnail,
        VariantSpec::Width(1000),
        VariantSpec::Full,
    ];

    /// The SDK variant.
    pub fn to_image_variant(self) -> ImageVariant {
        match self {
            VariantSpec::HighRes => ImageVariant::Preview(PreviewSize::HighRes),
            VariantSpec::MedRes => ImageVariant::Preview(PreviewSize::MedRes),
            VariantSpec::LowRes => ImageVariant::Preview(PreviewSize::LowRes),
            VariantSpec::Thumbnail => ImageVariant::Preview(PreviewSize::Thumbnail),
            VariantSpec::Width(w) => ImageVariant::width(w),
            VariantSpec::Full => ImageVariant::full(),
        }
    }

    /// The quota class.
    pub fn quota_class(self) -> QuotaClass {
        self.to_image_variant().quota_class()
    }

    /// `true` for the strictest quota (5 per minute).
    pub fn is_full_resolution(self) -> bool {
        self.to_image_variant().is_full_resolution()
    }

    /// Human description.
    pub fn description(self) -> String {
        match self {
            VariantSpec::HighRes => "highres preview, fits 1024×1536 px, 50/min".into(),
            VariantSpec::MedRes => "medres preview, fits 512×768 px".into(),
            VariantSpec::LowRes => "lowres preview, fits 256×384 px".into(),
            VariantSpec::Thumbnail => "thumbnail, fits 128×192 px".into(),
            VariantSpec::Width(w) if w > 1000 => format!("IIIF {w} px wide, 5/min"),
            VariantSpec::Width(w) => format!("IIIF {w} px wide"),
            VariantSpec::Full => "full resolution (~4300×6000 px), 5/min".into(),
        }
    }
}

impl fmt::Display for VariantSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariantSpec::HighRes => f.write_str("highres"),
            VariantSpec::MedRes => f.write_str("medres"),
            VariantSpec::LowRes => f.write_str("lowres"),
            VariantSpec::Thumbnail => f.write_str("thumbnail"),
            VariantSpec::Width(w) => write!(f, "width:{w}"),
            VariantSpec::Full => f.write_str("full"),
        }
    }
}

impl FromStr for VariantSpec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "highres" => Ok(VariantSpec::HighRes),
            "medres" => Ok(VariantSpec::MedRes),
            "lowres" => Ok(VariantSpec::LowRes),
            "thumbnail" => Ok(VariantSpec::Thumbnail),
            "full" => Ok(VariantSpec::Full),
            other => match other
                .strip_prefix("width:")
                .or_else(|| other.strip_prefix("width="))
            {
                Some(w) => w
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|w| *w > 0)
                    .map(VariantSpec::Width)
                    .ok_or_else(|| format!("invalid width in {s:?}")),
                None => Err(format!(
                    "unknown variant {s:?}: expected highres, medres, lowres, thumbnail, width:<px> or full"
                )),
            },
        }
    }
}

impl Serialize for VariantSpec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for VariantSpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_spec_roundtrip() {
        for v in VariantSpec::CHOICES {
            assert_eq!(v.to_string().parse::<VariantSpec>().unwrap(), v);
        }
        assert_eq!(
            "WIDTH:800".parse::<VariantSpec>().unwrap(),
            VariantSpec::Width(800)
        );
        assert!("width:0".parse::<VariantSpec>().is_err());
        assert!("huge".parse::<VariantSpec>().is_err());
        assert!(VariantSpec::Full.is_full_resolution());
        assert!(VariantSpec::Width(1200).is_full_resolution());
        assert!(!VariantSpec::Width(1000).is_full_resolution());
        assert_eq!(
            VariantSpec::HighRes.quota_class(),
            QuotaClass::HighResPreview
        );
    }

    #[test]
    fn settings_toml_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        assert_eq!(Settings::load_from(&path).unwrap(), Settings::default());
        let s = Settings {
            contact: Some("me@example.org".into()),
            variant: VariantSpec::Width(900),
            years: Some((1900, 1910)),
            quotas: Some(Quotas::unlimited()),
            ..Settings::default()
        };
        s.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("variant = \"width:900\""), "{text}");
        assert_eq!(Settings::load_from(&path).unwrap(), s);
        assert!(s.user_agent().ends_with("contact:me@example.org"));
        assert!(!Settings::default().has_contact());
    }

    #[test]
    fn environment_overrides_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::write(&path, "contact = \"file@example.org\"\noverwrite = true\n").unwrap();
        let env = |vars: &[(&str, &str)]| {
            Environment::with_prefix(ENV_PREFIX).source(Some(
                vars.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ))
        };
        let s = Settings::load_with(&path, env(&[])).unwrap();
        assert_eq!(s.contact.as_deref(), Some("file@example.org"));
        assert!(s.overwrite);

        let s = Settings::load_with(
            &path,
            env(&[
                ("GALLICA_CONTACT", "env@example.org"),
                ("GALLICA_OUTPUT_DIR", "/tmp/out"),
                ("GALLICA_VARIANT", "width:800"),
                ("GALLICA_MAX_ISSUES", "3"),
                ("GALLICA_QUOTAS__TEXTS_PER_MINUTE", "10"),
            ]),
        )
        .unwrap();
        assert_eq!(s.contact.as_deref(), Some("env@example.org"));
        assert_eq!(s.output_dir, PathBuf::from("/tmp/out"));
        assert_eq!(s.variant, VariantSpec::Width(800));
        assert_eq!(s.max_issues, Some(3));
        assert_eq!(s.quotas.unwrap().texts_per_minute, Some(10));
        assert!(s.overwrite, "the file still applies");

        let err = Settings::load_with(&path, env(&[("GALLICA_VARIANT", "huge")])).unwrap_err();
        assert!(err.to_string().contains("unknown variant"), "{err}");
    }
}
