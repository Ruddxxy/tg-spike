//! Fetch and paginate the Telegraph daemon question feed.

use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::cache::HttpCache;
use crate::error::BuildError;

/// Page size used for every feed request.
const PAGE_LIMIT: u64 = 100;

/// Delay between feed page requests that actually hit the network. Cache
/// hits are not throttled: politeness only matters when a real request
/// goes out.
const PAGE_SLEEP: Duration = Duration::from_millis(300);

/// One page of the `/daemon/api/questions` feed.
#[derive(Deserialize)]
struct FeedPage {
    results: Vec<QuestionEntry>,
    total: u64,
}

/// One entry ("row") from the question feed.
///
/// Only the fields this crate needs are named. Unknown fields are ignored
/// by serde's default behaviour, so the daemon can add fields without
/// breaking this parser.
#[derive(Deserialize, Clone, Debug)]
pub struct QuestionEntry {
    /// Feed-assigned row id. Stable within one feed snapshot; used to key
    /// cluster assignment back onto a response.
    pub id: String,
    /// The daemon's reported status for this row: `"success"` or
    /// `"error"`. `execution.result` can be JSON null under EITHER
    /// status, so null must be checked directly, not inferred from this
    /// field.
    pub status: String,
    pub question: QuestionInfo,
    #[serde(default)]
    pub routing: RoutingInfo,
    pub execution: ExecutionInfo,
}

/// The question text and identity fields for one feed row.
#[derive(Deserialize, Clone, Debug)]
pub struct QuestionInfo {
    pub text: String,
}

/// Routing metadata for one feed row. `miner_slug` and `intent` are
/// absent on user-directed `"[direct] N -> /path"` rows.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct RoutingInfo {
    pub miner_slug: Option<String>,
    pub intent: Option<String>,
}

/// The execution outcome for one feed row.
#[derive(Deserialize, Clone, Debug)]
pub struct ExecutionInfo {
    /// The raw miner response, captured as the EXACT byte span from the
    /// cached feed body, not a re-serialisation. `RawValue` is
    /// `serde_json`'s deferred-parse type: it records the original
    /// slice of input text for this JSON value during the top-level
    /// parse, instead of building a `Value` tree and re-printing it
    /// later. This is what ends up in the corpus's `miner_answer`
    /// field, so a later scoring wave gets the exact bytes the daemon
    /// recorded, not this crate's idea of how to format them.
    ///
    /// The literal text `"null"` when the daemon recorded no result.
    pub result: Box<RawValue>,
}

/// Fetch every page of the question feed, following `total` from the
/// server's first response.
///
/// Returns all entries plus the server-reported `total` row count. A
/// network failure on any page propagates as `Err` (never a panic); the
/// caller decides whether that aborts the run.
pub fn fetch_all(
    cache: &mut HttpCache,
    base_url: &str,
) -> Result<(Vec<QuestionEntry>, u64), BuildError> {
    let mut offset = 0u64;
    let mut all = Vec::new();
    let mut total = u64::MAX;

    while offset < total {
        let url = format!("{base_url}?limit={PAGE_LIMIT}&offset={offset}");
        let before = cache.network_requests();
        let body = cache.fetch(&url)?;
        let page: FeedPage = serde_json::from_str(&body)
            .map_err(|e| BuildError::Json(format!("feed page at offset {offset}: {e}")))?;

        total = page.total;
        all.extend(page.results);
        offset += PAGE_LIMIT;

        if cache.network_requests() > before && offset < total {
            thread::sleep(PAGE_SLEEP);
        }
    }

    Ok((all, total))
}
