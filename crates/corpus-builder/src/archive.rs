//! Open-Meteo historical archive: ground-truth actual temperatures.
//!
//! Requests are grouped by rounded (lat, lon) so one location with many
//! valid times costs one request, covering the whole date span needed at
//! that location.
//!
//! A discovered wrinkle: the archive's valid coverage window ends at
//! "today" server-side, and requesting a date range that extends past
//! that window does not fail at the transport level. It returns HTTP 200
//! with a body shaped `{"error":true,"reason":"Parameter 'end_date' is
//! out of allowed range from 1940-01-01 to 2026-08-13"}`. Since a single
//! location's points can span both past (real) and future (forecast)
//! valid times, this module parses that reason, clamps the range to the
//! server-reported max date, and retries once (still cached) rather than
//! treating the whole group as failed. A location whose entire range is
//! in the future gets zero archive coverage, which is correct, not a
//! failure: those points legitimately have no actual yet.

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use crate::cache::HttpCache;
use crate::error::BuildError;
use crate::schema::WeatherPoint;

const ARCHIVE_BASE: &str = "https://archive-api.open-meteo.com/v1/archive";

/// Delay between archive requests that actually hit the network.
const REQUEST_SLEEP: Duration = Duration::from_millis(300);

/// Rounding applied to lat/lon before grouping requests, expressed as a
/// multiplier: `round(v * GROUP_PRECISION) / GROUP_PRECISION`. 100.0
/// rounds to 2 decimal places (about 1 km at the equator), so points a
/// short walk apart share one archive request.
const GROUP_PRECISION: f64 = 100.0;

/// The unit the archive is expected to declare for `temperature_2m`.
/// Asserted, never assumed: a response declaring anything else is
/// treated as a failure for that group, not silently trusted.
const EXPECTED_UNIT: &str = "°C";

#[derive(Deserialize)]
#[serde(untagged)]
enum ArchiveResult {
    Ok(ArchiveResponse),
    Error { reason: String },
}

#[derive(Deserialize)]
struct ArchiveResponse {
    hourly: ArchiveHourly,
    hourly_units: ArchiveUnits,
}

#[derive(Deserialize)]
struct ArchiveHourly {
    time: Vec<String>,
    temperature_2m: Vec<Option<f64>>,
}

#[derive(Deserialize)]
struct ArchiveUnits {
    temperature_2m: String,
}

/// Ground-truth actuals, keyed by `(round(lat*100), round(lon*100),
/// naive minute-precision time string)`.
pub struct ActualIndex {
    values: HashMap<(i64, i64, String), f64>,
    /// Location groups where the archive fetch failed for a reason other
    /// than "out of coverage" (network failure, wrong declared unit,
    /// mismatched array lengths). Every point in these groups has no
    /// actual, and is counted as a failure, not guessed.
    pub failed_groups: Vec<((i64, i64), String)>,
}

fn round_key(v: f64) -> i64 {
    (v * GROUP_PRECISION).round() as i64
}

impl ActualIndex {
    /// Look up the actual Celsius temperature for one point, if the
    /// archive covers that instant.
    pub fn lookup(&self, lat: f64, lon: f64, valid_time_utc: &str) -> Option<f64> {
        let key = (round_key(lat), round_key(lon), valid_time_utc.to_string());
        self.values.get(&key).copied()
    }

    /// An index with no coverage at all, for tests in other modules that
    /// need an `ActualIndex` but do not exercise archive fetching.
    #[cfg(test)]
    pub fn empty_for_tests() -> Self {
        ActualIndex {
            values: HashMap::new(),
            failed_groups: Vec::new(),
        }
    }
}

/// Parse the max allowed date out of an Open-Meteo "out of allowed range"
/// reason string, e.g. `"...from 1940-01-01 to 2026-08-13"` -> the
/// substring `"2026-08-13"`.
fn parse_max_date(reason: &str) -> Option<&str> {
    let date = reason.rsplit(" to ").next()?;
    let bytes = date.as_bytes();
    let looks_like_date =
        date.len() == 10 && bytes.get(4) == Some(&b'-') && bytes.get(7) == Some(&b'-');
    looks_like_date.then_some(date)
}

fn build_url(lat: f64, lon: f64, start: &str, end: &str) -> String {
    format!("{ARCHIVE_BASE}?latitude={lat:.2}&longitude={lon:.2}&start_date={start}&end_date={end}&hourly=temperature_2m&timezone=UTC")
}

fn timed_fetch(cache: &mut HttpCache, url: &str) -> Result<String, BuildError> {
    let before = cache.network_requests();
    let body = cache.fetch(url)?;
    if cache.network_requests() > before {
        thread::sleep(REQUEST_SLEEP);
    }
    Ok(body)
}

/// Fetch one location's archive coverage for `[start, end]`, transparently
/// clamping to the server's coverage window when the range reaches into
/// the future.
///
/// Returns `Ok(None)` when the location has legitimately zero archive
/// coverage in range (a pure-future request); `Ok(Some(_))` with whatever
/// coverage exists otherwise; `Err` only for a genuine failure (network,
/// bad JSON, wrong unit, mismatched lengths).
fn fetch_range(
    cache: &mut HttpCache,
    lat: f64,
    lon: f64,
    start: &str,
    end: &str,
) -> Result<Option<ArchiveResponse>, BuildError> {
    let url = build_url(lat, lon, start, end);
    let body = timed_fetch(cache, &url)?;
    let parsed: ArchiveResult = serde_json::from_str(&body)
        .map_err(|e| BuildError::Json(format!("archive response for {url}: {e}")))?;

    let reason = match parsed {
        ArchiveResult::Ok(resp) => return validate(resp, &url),
        ArchiveResult::Error { reason } => reason,
    };

    let Some(max_date) = parse_max_date(&reason) else {
        return Err(BuildError::Http(format!(
            "archive error for {url}: {reason}"
        )));
    };

    if reason.contains("'start_date'") && start > max_date {
        // The whole range is beyond the archive's coverage window: no
        // failure, just no data yet.
        return Ok(None);
    }

    if end > max_date {
        if start > max_date {
            return Ok(None);
        }
        let clamped_url = build_url(lat, lon, start, max_date);
        let clamped_body = timed_fetch(cache, &clamped_url)?;
        let clamped_parsed: ArchiveResult = serde_json::from_str(&clamped_body)
            .map_err(|e| BuildError::Json(format!("archive response for {clamped_url}: {e}")))?;
        return match clamped_parsed {
            ArchiveResult::Ok(resp) => validate(resp, &clamped_url),
            ArchiveResult::Error { reason } => Err(BuildError::Http(format!(
                "archive still errors after clamping end_date for {clamped_url}: {reason}"
            ))),
        };
    }

    Err(BuildError::Http(format!(
        "archive error for {url}: {reason}"
    )))
}

fn validate(resp: ArchiveResponse, url: &str) -> Result<Option<ArchiveResponse>, BuildError> {
    if resp.hourly_units.temperature_2m != EXPECTED_UNIT {
        return Err(BuildError::Http(format!(
            "archive response for {url} declared unit {:?}, expected {EXPECTED_UNIT:?}",
            resp.hourly_units.temperature_2m
        )));
    }
    if resp.hourly.time.len() != resp.hourly.temperature_2m.len() {
        return Err(BuildError::Http(format!(
            "archive response for {url} has mismatched hourly array lengths"
        )));
    }
    Ok(Some(resp))
}

/// Fetch (with caching) every Open-Meteo archive page needed to cover
/// `points`, grouped by rounded (lat, lon) to keep the request count low.
pub fn build_index(
    cache: &mut HttpCache,
    points: &[&WeatherPoint],
) -> Result<ActualIndex, BuildError> {
    // Group required dates by rounded (lat, lon).
    let mut groups: HashMap<(i64, i64), (f64, f64, String, String)> = HashMap::new();
    for p in points {
        let key = (round_key(p.lat), round_key(p.lon));
        let date = &p.valid_time_utc[..10.min(p.valid_time_utc.len())];
        groups
            .entry(key)
            .and_modify(|(_, _, start, end)| {
                if date < start.as_str() {
                    *start = date.to_string();
                }
                if date > end.as_str() {
                    *end = date.to_string();
                }
            })
            .or_insert((p.lat, p.lon, date.to_string(), date.to_string()));
    }

    let mut ordered: Vec<_> = groups.into_iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));

    let mut values = HashMap::new();
    let mut failed_groups = Vec::new();

    for (key, (_raw_lat, _raw_lon, start, end)) in ordered {
        let rounded_lat = key.0 as f64 / GROUP_PRECISION;
        let rounded_lon = key.1 as f64 / GROUP_PRECISION;

        match fetch_range(cache, rounded_lat, rounded_lon, &start, &end) {
            Ok(Some(resp)) => {
                for (time, temp) in resp
                    .hourly
                    .time
                    .iter()
                    .zip(resp.hourly.temperature_2m.iter())
                {
                    if let Some(t) = temp {
                        values.insert((key.0, key.1, time.clone()), *t);
                    }
                }
            }
            Ok(None) => {
                // Legitimately no coverage (pure-future range). Not a
                // failure: nothing to record.
            }
            Err(err) => {
                failed_groups.push((key, err.to_string()));
            }
        }
    }

    Ok(ActualIndex {
        values,
        failed_groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_max_date_from_reason_string() {
        assert_eq!(
            parse_max_date(
                "Parameter 'end_date' is out of allowed range from 1940-01-01 to 2026-08-13"
            ),
            Some("2026-08-13")
        );
        assert_eq!(
            parse_max_date(
                "Parameter 'start_date' is out of allowed range from 1940-01-01 to 2026-08-13"
            ),
            Some("2026-08-13")
        );
    }

    #[test]
    fn rejects_reason_without_a_trailing_date() {
        assert_eq!(parse_max_date("some other error"), None);
    }

    #[test]
    fn round_key_groups_nearby_coordinates() {
        assert_eq!(round_key(25.7501), round_key(25.7499));
    }

    #[test]
    fn round_key_separates_distant_coordinates() {
        assert_ne!(round_key(25.75), round_key(26.75));
    }
}
