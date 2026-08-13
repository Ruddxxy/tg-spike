//! Shared error type for the corpus builder.
//!
//! This is a host-side tool, not the scoring module, so a plain enum with
//! `Display` is enough. No `unwrap`/`expect` on this path: every fallible
//! call returns `Result<_, BuildError>` and the caller decides whether to
//! stop the run or just record the failure and move on.

use std::fmt;

/// An error from any stage of the corpus build.
#[derive(Debug)]
pub enum BuildError {
    /// A network request failed after retries were exhausted.
    Http(String),
    /// `--offline` was set and the URL was not already cached.
    OfflineMiss(String),
    /// A filesystem operation failed.
    Io(std::io::Error),
    /// A JSON body did not parse, or did not match the expected shape.
    Json(String),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::Http(msg) => write!(f, "network error: {msg}"),
            BuildError::OfflineMiss(url) => write!(
                f,
                "offline mode: no cached response for {url} (run once without --offline first)"
            ),
            BuildError::Io(err) => write!(f, "I/O error: {err}"),
            BuildError::Json(msg) => write!(f, "JSON error: {msg}"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<std::io::Error> for BuildError {
    fn from(err: std::io::Error) -> Self {
        BuildError::Io(err)
    }
}

impl From<serde_json::Error> for BuildError {
    fn from(err: serde_json::Error) -> Self {
        BuildError::Json(err.to_string())
    }
}
