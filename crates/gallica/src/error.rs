//! Error types for the Gallica client.

use std::fmt;

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors produced by the Gallica client.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The input could not be parsed as an ARK identifier.
    #[error("invalid ARK: {0}")]
    Ark(#[from] ArkError),

    /// The server answered with a non-success HTTP status.
    #[error("HTTP {status} for {url}{}", body_hint(.body))]
    Http {
        /// The requested URL.
        url: String,
        /// The HTTP status code.
        status: u16,
        /// The first bytes of the body, if it was text.
        body: Option<String>,
    },

    /// The requested resource does not exist (HTTP 404 or an empty service answer).
    #[error("not found: {0}")]
    NotFound(String),

    /// A transport level error (DNS, TLS, connection reset, timeout...).
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),

    /// The server answered with XML that could not be parsed or that did not
    /// contain what we expected.
    #[error("malformed XML in {context}: {message}")]
    Xml {
        /// What was being parsed.
        context: &'static str,
        /// Details.
        message: String,
    },

    /// The server answered with JSON that could not be parsed.
    #[error("malformed JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// An invalid URL was built or supplied.
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),

    /// A local filesystem error while saving a download.
    #[error("I/O error{}: {source}", path_hint(.path))]
    Io {
        /// The path involved, if any.
        path: Option<std::path::PathBuf>,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// A caller supplied an argument the API cannot accept.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// All retries were exhausted for a request.
    #[error("gave up after {attempts} attempts on {url}: {last}")]
    RetriesExhausted {
        /// The requested URL.
        url: String,
        /// How many attempts were made.
        attempts: u32,
        /// The last error seen.
        #[source]
        last: Box<Error>,
    },
}

impl Error {
    pub(crate) fn xml(context: &'static str, message: impl fmt::Display) -> Self {
        Error::Xml {
            context,
            message: message.to_string(),
        }
    }

    pub(crate) fn io(path: Option<std::path::PathBuf>, source: std::io::Error) -> Self {
        Error::Io { path, source }
    }

    /// Returns `true` if the error is transient and retrying may succeed.
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Http { status, .. } => matches!(*status, 408 | 429 | 500 | 502 | 503 | 504),
            Error::Transport(e) => e.is_timeout() || e.is_connect() || e.is_request(),
            _ => false,
        }
    }

    /// The HTTP status code, if this error carries one.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Http { status, .. } => Some(*status),
            Error::Transport(e) => e.status().map(|s| s.as_u16()),
            Error::RetriesExhausted { last, .. } => last.status(),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Io { path: None, source }
    }
}

fn body_hint(body: &Option<String>) -> String {
    match body {
        Some(b) if !b.trim().is_empty() => format!(": {}", b.trim()),
        _ => String::new(),
    }
}

fn path_hint(path: &Option<std::path::PathBuf>) -> String {
    match path {
        Some(p) => format!(" ({})", p.display()),
        None => String::new(),
    }
}

/// Errors produced while parsing an ARK identifier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ArkError {
    /// The input was empty.
    #[error("empty input")]
    Empty,
    /// The host is not a Gallica host.
    #[error("unsupported host {0:?}: only gallica.bnf.fr URLs are accepted")]
    UnsupportedHost(String),
    /// The name assigning authority number is not the BnF one.
    #[error("unsupported NAAN {0:?}: only 12148 (BnF) is accepted")]
    UnsupportedNaan(String),
    /// The ARK name is missing or contains invalid characters.
    #[error("invalid ARK name {0:?}")]
    InvalidName(String),
    /// The input does not look like an ARK or a Gallica URL at all.
    #[error("cannot find an ARK in {0:?}")]
    Unrecognized(String),
}
