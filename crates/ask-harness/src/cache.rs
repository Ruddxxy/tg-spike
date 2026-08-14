//! This module caches every response to disk.
//!
//! An ask costs money. A cached response makes the analysis replayable
//! offline, so a rerun never spends again.
//!
//! The cache holds RESPONSES only. It never holds the private key, and
//! it never holds the payment header, because that header carries a
//! signature that authorises a transfer.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// The directory that holds the cached responses.
pub const CACHE_DIRECTORY: &str = "corpus/ask-cache";

/// This function writes one response to the cache.
///
/// The file name carries the run label and the time, so two asks of
/// the same question never overwrite each other.
pub fn store(label: &str, question: &str, body: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(CACHE_DIRECTORY)
        .map_err(|error| format!("cannot make the cache directory: {error}"))?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "the system clock is before the epoch".to_string())?
        .as_millis();

    let safe_label: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();

    let path = PathBuf::from(CACHE_DIRECTORY).join(format!("{safe_label}-{stamp}.json"));
    let record = serde_json::json!({
        "label": label,
        "question": question,
        "unix_millis": stamp.to_string(),
        "body": body,
    });

    let mut file =
        fs::File::create(&path).map_err(|error| format!("cannot make the cache file: {error}"))?;
    file.write_all(record.to_string().as_bytes())
        .map_err(|error| format!("cannot write the cache file: {error}"))?;
    Ok(path)
}
