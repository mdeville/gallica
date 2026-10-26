//! Command line definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use gallica_core::{Settings, VariantSpec};

/// Bulk downloader for Gallica, the digital library of the BnF.
#[derive(Debug, Parser)]
#[command(name = "gallica", version, about, long_about = None)]
pub struct Cli {
    /// Settings file (default: the platform configuration directory).
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// More output (-v: info, -vv: debug).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(flatten)]
    pub overrides: Overrides,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Settings that can be overridden for one invocation.
#[derive(Debug, Clone, Default, Args)]
pub struct Overrides {
    /// Contact address sent to Gallica in the User-Agent.
    #[arg(long, global = true, value_name = "EMAIL")]
    pub contact: Option<String>,

    /// Output directory.
    #[arg(long, short = 'o', global = true, value_name = "DIR")]
    pub out: Option<PathBuf>,

    /// Image variant: highres, medres, lowres, thumbnail, width:<px> or full.
    #[arg(long, global = true, value_name = "VARIANT")]
    pub variant: Option<VariantSpec>,

    /// Year range for a periodical, e.g. 1921-1925 or 1921.
    #[arg(long, global = true, value_name = "FROM-TO", value_parser = parse_years)]
    pub years: Option<(u16, u16)>,

    /// Keep only the first N issues (test runs).
    #[arg(long, global = true, value_name = "N")]
    pub max_issues: Option<usize>,

    /// Overwrite existing files.
    #[arg(long, global = true)]
    pub overwrite: bool,

    /// Also save the plain text of each issue (5 per minute quota).
    #[arg(long, global = true)]
    pub save_text: bool,

    /// Do not ask for confirmation (needed for full resolution downloads).
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,

    /// Gallica base URL (tests and mirrors).
    #[arg(long, global = true, hide = true, value_name = "URL")]
    pub base_url: Option<String>,

    /// Minimum delay between requests in milliseconds.
    #[arg(long, global = true, hide = true, value_name = "MS")]
    pub min_interval_ms: Option<u64>,

    /// Disable the BnF quotas (tests only).
    #[arg(long, global = true, hide = true)]
    pub no_quotas: bool,
}

impl Overrides {
    /// Applies the overrides to loaded settings.
    pub fn apply(&self, settings: &mut Settings) {
        if let Some(c) = &self.contact {
            settings.contact = Some(c.clone());
        }
        if let Some(o) = &self.out {
            settings.output_dir = o.clone();
        }
        if let Some(v) = self.variant {
            settings.variant = v;
        }
        if let Some(y) = self.years {
            settings.years = Some(y);
        }
        if let Some(m) = self.max_issues {
            settings.max_issues = Some(m);
        }
        if self.overwrite {
            settings.overwrite = true;
        }
        if self.save_text {
            settings.save_text = true;
        }
        if let Some(u) = &self.base_url {
            settings.base_url = Some(u.clone());
        }
        if let Some(ms) = self.min_interval_ms {
            settings.min_request_interval_ms = Some(ms);
        }
        if self.no_quotas {
            settings.quotas = Some(gallica_core::gallica::Quotas::unlimited());
        }
    }
}

/// Subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Resolve a URL or ARK and print what it is.
    Info {
        /// Gallica URL or ARK.
        target: String,
    },
    /// Print what a download would fetch and what it would cost.
    Plan {
        /// Gallica URL or ARK.
        target: String,
    },
    /// Download the target.
    Download {
        /// Gallica URL or ARK.
        target: String,
    },
    /// Open the terminal UI.
    Tui {
        /// Gallica URL or ARK to start with.
        target: Option<String>,
    },
}

fn parse_years(s: &str) -> Result<(u16, u16), String> {
    let s = s.trim();
    let (a, b) = match s.split_once('-') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (s, s),
    };
    let a: u16 = a.parse().map_err(|_| format!("invalid year {a:?}"))?;
    let b: u16 = b.parse().map_err(|_| format!("invalid year {b:?}"))?;
    Ok(if a <= b { (a, b) } else { (b, a) })
}

/// Loads the settings file and applies the overrides.
pub fn load_settings(cli: &Cli) -> Result<Settings, String> {
    let mut settings = match &cli.config {
        Some(path) => Settings::load_from(path).map_err(|e| e.to_string())?,
        None => Settings::load().map_err(|e| e.to_string())?,
    };
    cli.overrides.apply(&mut settings);
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_years() {
        assert_eq!(parse_years("1921-1925").unwrap(), (1921, 1925));
        assert_eq!(parse_years("1925 - 1921").unwrap(), (1921, 1925));
        assert_eq!(parse_years("1921").unwrap(), (1921, 1921));
        assert!(parse_years("abc").is_err());
    }

    #[test]
    fn parses_command_line() {
        let cli = Cli::try_parse_from([
            "gallica",
            "download",
            "cb327110547",
            "--years",
            "1921",
            "--variant",
            "width:800",
            "-y",
            "--out",
            "/tmp/x",
        ])
        .unwrap();
        assert!(matches!(cli.command, Some(Command::Download { .. })));
        assert_eq!(cli.overrides.variant, Some(VariantSpec::Width(800)));
        assert_eq!(cli.overrides.years, Some((1921, 1921)));
        assert!(cli.overrides.yes);
        let mut s = Settings::default();
        cli.overrides.apply(&mut s);
        assert_eq!(s.output_dir, PathBuf::from("/tmp/x"));
        assert_eq!(s.variant, VariantSpec::Width(800));
    }
}
