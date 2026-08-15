//! This module joins ground truth onto the bought asks and emits a
//! corpus file the rest of the pipeline reads unchanged.
//!
//! ## The two timestamps, which are NOT the same
//!
//! A row carries two times and they must not be confused.
//!
//! - `valid_time` is the miner's OWN claimed observation time, taken
//!   from its response by the corpus normaliser. It exists so that
//!   `corpus-eval prepare` can find the right point inside a response
//!   that holds many: `prepare` matches a point by exact time string.
//!   Setting this field to anything else drops every row from a miner
//!   whose reading is not on the hour, which is both live weather
//!   miners.
//! - `archive_hour` is the hour NEAREST THE ASK, computed from the
//!   client-side timestamp taken before the request was sent. The
//!   archive actual is looked up at this hour, at the city's own
//!   coordinates.
//!
//! The truth is therefore joined at the time the client asked and the
//! place the client named, never at anything a miner reported. That is
//! the daemon-feed corpus defect being designed out: it joined at
//! miner-returned coordinates, so a miner answering for the wrong city
//! was scored against that wrong city's weather and the error was
//! undetectable.
//!
//! The gap between the two times is measured and reported. A miner
//! serving a stale reading shows up there.
//!
//! ## Extra fields are safe
//!
//! The emitted rows carry more fields than `CorpusRow` reads. Serde
//! ignores unknown fields, so `prepare` consumes this file unchanged
//! while the provenance stays in the record.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use corpus_builder::rounding::{format_temp_c, round_temp_c};
use corpus_builder::schema;

use crate::geocode::Coordinates;

/// The Open-Meteo archive endpoint.
const ARCHIVE_BASE: &str = "https://archive-api.open-meteo.com/v1/archive";

/// The unit the archive must declare. Asserted, never assumed.
const EXPECTED_UNIT: &str = "°C";

/// Where the joined corpus is written.
pub const OUTPUT_PATH: &str = "corpus/head-to-head.jsonl";

/// The miner id to slug map, from the node's own registry.
///
/// `GET /api/miners?intent=WEATHER_CHECK` is the source. The slug is
/// what the corpus normaliser dispatches on, and the ask response gives
/// only the id and the display name.
const MINER_SLUGS: [(&str, &str); 5] = [
    ("211", "openweathermap"),
    ("212", "weatherapi"),
    ("18", "bittensor-sn18-zeus"),
    ("0", "lacre-meteo"),
    ("64173", "oathcast-weather"),
];

/// The `label_field` each miner's registry entry points at.
///
/// Three of these name an OBJECT or an ARRAY, not a scalar. The scoring
/// module receives one extracted value and its parser assumes a scalar,
/// so a standardiser that passed the labelled field through unchanged
/// would hand the module a JSON blob. This table exists so the report
/// can state which miners are affected.
const LABEL_FIELDS: [(&str, &str, &str); 5] = [
    ("211", "weather", "array of objects"),
    ("212", "current", "object"),
    ("18", "model", "scalar (string)"),
    ("0", "hourly", "object of arrays"),
    ("64173", "content", "scalar (text)"),
];

/// This function gives the slug for a miner id.
pub fn slug_for(miner_id: &str) -> Option<&'static str> {
    MINER_SLUGS
        .iter()
        .find(|(id, _)| *id == miner_id)
        .map(|(_, slug)| *slug)
}

/// This function describes the label field of a miner id.
pub fn label_field_for(miner_id: &str) -> Option<(&'static str, &'static str)> {
    LABEL_FIELDS
        .iter()
        .find(|(id, _, _)| *id == miner_id)
        .map(|(_, field, shape)| (*field, *shape))
}

/// One line of the bought-ask record.
#[derive(Debug, Deserialize)]
pub struct AskLine {
    /// The position in the run.
    pub index: usize,
    /// The city key, which becomes the cluster id.
    pub city_key: String,
    /// The exact query string sent.
    pub query: String,
    /// The client-side ask time, in Unix seconds.
    pub asked_at_unix: i64,
    /// The HTTP status.
    pub status: u16,
    /// Whether the node settled the payment.
    pub settled: bool,
    /// The miner id that answered.
    pub miner_id: Option<String>,
    /// The miner display name.
    pub miner_name: Option<String>,
    /// The intent the router chose.
    pub intent: Option<String>,
    /// A failure note.
    pub failure: Option<String>,
    /// The raw response body.
    pub body: String,
}

/// This function reads the bought-ask record.
pub fn load_asks(path: &Path) -> Result<Vec<AskLine>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut rows = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: AskLine = serde_json::from_str(line).map_err(|error| {
            format!(
                "line {} of the ask record is unreadable: {error}",
                number + 1
            )
        })?;
        rows.push(row);
    }
    Ok(rows)
}

/// This function turns Unix seconds into the archive's hour key.
///
/// The hour is the NEAREST one, not the floor. A reading taken at
/// 09:50 belongs to the 10:00 slot, and flooring would compare it
/// against an hour-old actual.
pub fn nearest_archive_hour(unix_seconds: i64) -> String {
    let rounded = ((unix_seconds + 1800) / 3600) * 3600;
    format_naive_hour(rounded)
}

/// This function renders Unix seconds as a naive UTC hour string.
fn format_naive_hour(unix_seconds: i64) -> String {
    // Civil-from-days, so this module needs no date library and cannot
    // disagree with the archive's own formatting.
    let days = unix_seconds.div_euclid(86400);
    let seconds_of_day = unix_seconds.rem_euclid(86400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3600;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:00")
}

/// This function converts days since the epoch to a civil date.
///
/// The algorithm is Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// This function gives the date part of an hour key.
fn date_of(hour_key: &str) -> &str {
    hour_key.split('T').next().unwrap_or(hour_key)
}

/// The archive coverage for one location.
pub struct ArchiveSeries {
    /// Hour key to Celsius actual.
    values: BTreeMap<String, f64>,
}

impl ArchiveSeries {
    /// This function looks up one hour.
    pub fn lookup(&self, hour_key: &str) -> Option<f64> {
        self.values.get(hour_key).copied()
    }
}

/// This function fetches the archive for one location and date span.
///
/// This is a smaller fetcher than the one in `corpus-builder`, which
/// carries the builder's own HTTP cache and grouping. The unit
/// assertion is kept, because a response that declares another unit
/// must fail rather than be trusted.
pub fn fetch_archive(
    latitude: f64,
    longitude: f64,
    start_date: &str,
    end_date: &str,
) -> Result<ArchiveSeries, String> {
    let url = format!(
        "{ARCHIVE_BASE}?latitude={latitude:.4}&longitude={longitude:.4}\
         &start_date={start_date}&end_date={end_date}&hourly=temperature_2m&timezone=UTC"
    );
    let body = ureq::get(&url)
        .call()
        .map_err(|error| format!("the archive request failed: {error}"))?
        .into_string()
        .map_err(|error| format!("cannot read the archive response: {error}"))?;

    let parsed: Value = serde_json::from_str(&body)
        .map_err(|error| format!("the archive response is not JSON: {error}"))?;

    if let Some(reason) = parsed.get("reason").and_then(Value::as_str) {
        return Err(format!("the archive refused the request: {reason}"));
    }

    let unit = parsed
        .pointer("/hourly_units/temperature_2m")
        .and_then(Value::as_str)
        .ok_or_else(|| "the archive response declares no temperature unit".to_string())?;
    if unit != EXPECTED_UNIT {
        return Err(format!(
            "the archive declared unit {unit:?}, expected {EXPECTED_UNIT:?}"
        ));
    }

    let times = parsed
        .pointer("/hourly/time")
        .and_then(Value::as_array)
        .ok_or_else(|| "the archive response has no hourly times".to_string())?;
    let temperatures = parsed
        .pointer("/hourly/temperature_2m")
        .and_then(Value::as_array)
        .ok_or_else(|| "the archive response has no hourly temperatures".to_string())?;
    if times.len() != temperatures.len() {
        return Err("the archive hourly arrays have different lengths".to_string());
    }

    let mut values = BTreeMap::new();
    for (time, temperature) in times.iter().zip(temperatures.iter()) {
        if let (Some(key), Some(value)) = (time.as_str(), temperature.as_f64()) {
            values.insert(key.to_string(), value);
        }
    }
    Ok(ArchiveSeries { values })
}

/// One emitted row plus the facts the report needs.
pub struct JoinedRow {
    /// The city key.
    pub city_key: String,
    /// The miner id.
    pub miner_id: String,
    /// The miner slug, when the normaliser knows it.
    pub miner_slug: String,
    /// The miner's claimed Celsius value.
    pub miner_c: f64,
    /// The archive actual at the ask hour.
    pub actual_c: f64,
    /// How far the miner's claimed observation time sits from the ask
    /// hour, in minutes.
    pub drift_minutes: i64,
    /// The whole emitted JSON line.
    pub line: String,
}

/// Why an ask produced no row.
#[derive(Debug, Default)]
pub struct DropCounts {
    /// Counted by reason.
    pub reasons: BTreeMap<String, usize>,
}

impl DropCounts {
    /// This function counts one drop.
    fn bump(&mut self, reason: &str) {
        *self.reasons.entry(reason.to_string()).or_insert(0) += 1;
    }
}

/// This function joins the truth and builds every emittable row.
pub fn join(
    asks: &[AskLine],
    coordinates: &BTreeMap<String, Coordinates>,
    archives: &BTreeMap<String, ArchiveSeries>,
    drops: &mut DropCounts,
) -> Vec<JoinedRow> {
    let mut rows = Vec::new();

    for ask in asks {
        if ask.failure.is_some() || ask.status != 200 {
            drops.bump("ask_failed");
            continue;
        }
        let Some(miner_id) = ask.miner_id.clone() else {
            drops.bump("no_miner_id");
            continue;
        };
        let Some(place) = coordinates.get(&ask.city_key) else {
            drops.bump("city_not_geocoded");
            continue;
        };
        let Some(series) = archives.get(&ask.city_key) else {
            drops.bump("no_archive_for_city");
            continue;
        };

        let archive_hour = nearest_archive_hour(ask.asked_at_unix);
        let Some(actual_raw) = series.lookup(&archive_hour) else {
            drops.bump("archive_has_no_actual_at_the_ask_hour");
            continue;
        };

        let Some(slug) = slug_for(&miner_id) else {
            drops.bump("unknown_miner_id");
            continue;
        };

        let body: Value = match serde_json::from_str(&ask.body) {
            Ok(value) => value,
            Err(_) => {
                drops.bump("response_not_json");
                continue;
            }
        };
        // The corpus stores the miner's `result`, not the whole engine
        // envelope, so the normaliser is given the same shape it was
        // built against.
        let Some(result) = body.get("result") else {
            drops.bump("response_has_no_result");
            continue;
        };

        let points = match schema::normalise(slug, result) {
            Ok(points) => points,
            Err(reason) => {
                drops.bump(&format!("normalise_{}", reason.label()));
                continue;
            }
        };
        if points.is_empty() {
            drops.bump("normalise_gave_no_points");
            continue;
        }

        // Choose the point nearest the ask hour. A "current" response
        // holds one; a forecast response holds many.
        let target = hour_to_unix(&archive_hour);
        let Some(point) = points
            .iter()
            .min_by_key(|point| (hour_string_to_unix(&point.valid_time_utc) - target).abs())
        else {
            drops.bump("no_point_chosen");
            continue;
        };
        let drift_minutes = (hour_string_to_unix(&point.valid_time_utc) - target).abs() / 60;

        let rounded = round_temp_c(actual_raw);
        let bare = format_temp_c(actual_raw);
        let prose = format!("The temperature was {bare} C.");
        // `time` names the hour the TRUTH is from, which is the ask
        // hour, not the miner's claimed observation time.
        let gt_json =
            serde_json::json!({"temperature_2m": rounded, "time": archive_hour}).to_string();

        let line = serde_json::json!({
            // Fields CorpusRow reads.
            "question": ask.query,
            "gt_bare": bare,
            "gt_prose": prose,
            "gt_json": gt_json,
            "miner_answer": result.to_string(),
            "miner_slug": slug,
            "intent": ask.intent.clone().unwrap_or_default(),
            "valid_time": point.valid_time_utc,
            "lat": place.latitude,
            "lon": place.longitude,
            "actual_c": rounded,
            "cluster_id": ask.city_key,
            // Provenance. Serde ignores these when CorpusRow reads the
            // file, so they cost nothing and record how the row was made.
            "ask_index": ask.index,
            "asked_at_unix": ask.asked_at_unix,
            "archive_hour": archive_hour,
            "miner_id": miner_id,
            "miner_name": ask.miner_name.clone().unwrap_or_default(),
            "settled": ask.settled,
            "truth_source": "open-meteo archive at the geocoded city and the ask hour",
        })
        .to_string();

        rows.push(JoinedRow {
            city_key: ask.city_key.clone(),
            miner_id,
            miner_slug: slug.to_string(),
            miner_c: point.temp_c,
            actual_c: rounded,
            drift_minutes,
            line,
        });
    }

    rows
}

/// This function converts an hour key back to Unix seconds.
fn hour_to_unix(hour_key: &str) -> i64 {
    hour_string_to_unix(hour_key)
}

/// This function converts a naive minute-precision UTC string to Unix
/// seconds.
///
/// A string this cannot read gives 0, which only ever makes a point
/// look far from the target and so never wins the nearest-point choice.
fn hour_string_to_unix(text: &str) -> i64 {
    let bytes = text.as_bytes();
    if bytes.len() < 16 {
        return 0;
    }
    let number = |from: usize, to: usize| -> i64 { text[from..to].parse().unwrap_or(0) };
    let (year, month, day) = (number(0, 4), number(5, 7), number(8, 10));
    let (hour, minute) = (number(11, 13), number(14, 16));
    days_from_civil(year, month as u32, day as u32) * 86400 + hour * 3600 + minute * 60
}

/// This function converts a civil date to days since the epoch.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// This function writes the joined rows.
pub fn write_rows(rows: &[JoinedRow], path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot make the output directory: {error}"))?;
    }
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("cannot make the output file: {error}"))?;
    for row in rows {
        writeln!(file, "{}", row.line)
            .map_err(|error| format!("cannot write the output file: {error}"))?;
    }
    Ok(())
}

/// This function gives the distinct city keys of a set of asks.
pub fn city_keys(asks: &[AskLine]) -> BTreeSet<String> {
    asks.iter().map(|ask| ask.city_key.clone()).collect()
}

/// This function gives the date span the archive must cover.
pub fn date_span(asks: &[AskLine]) -> (String, String) {
    let mut earliest = i64::MAX;
    let mut latest = i64::MIN;
    for ask in asks {
        earliest = earliest.min(ask.asked_at_unix);
        latest = latest.max(ask.asked_at_unix);
    }
    if earliest == i64::MAX {
        return ("1970-01-01".to_string(), "1970-01-01".to_string());
    }
    (
        date_of(&nearest_archive_hour(earliest)).to_string(),
        date_of(&nearest_archive_hour(latest)).to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants_format_as_the_archive_does() {
        // Each pair was computed independently with Python's
        // datetime.fromtimestamp(..., timezone.utc), not with this
        // code, so a shared bug in the civil-date maths cannot make
        // this test agree with itself.
        for (unix, expected) in [
            (0_i64, "1970-01-01T00:00"),
            (1_767_225_600, "2026-01-01T00:00"),
            (1_786_784_400, "2026-08-15T09:00"),
            (1_786_863_600, "2026-08-16T07:00"),
        ] {
            assert_eq!(format_naive_hour(unix), expected, "for epoch {unix}");
        }
    }

    #[test]
    fn the_hour_conversion_round_trips() {
        for text in [
            "2026-08-15T09:00",
            "2026-01-01T00:00",
            "2026-12-31T23:00",
            "2024-02-29T12:00",
        ] {
            assert_eq!(format_naive_hour(hour_string_to_unix(text)), text);
        }
    }

    #[test]
    fn the_archive_hour_is_the_nearest_not_the_floor() {
        // 09:50 belongs to the 10:00 slot. Flooring would compare a
        // reading against an actual an hour old.
        let base = hour_string_to_unix("2026-08-15T09:00");
        assert_eq!(nearest_archive_hour(base + 50 * 60), "2026-08-15T10:00");
        assert_eq!(nearest_archive_hour(base + 10 * 60), "2026-08-15T09:00");
        assert_eq!(nearest_archive_hour(base + 30 * 60), "2026-08-15T10:00");
        assert_eq!(nearest_archive_hour(base + 29 * 60), "2026-08-15T09:00");
    }

    #[test]
    fn every_live_weather_miner_has_a_slug() {
        for id in ["211", "212", "18", "0", "64173"] {
            assert!(slug_for(id).is_some(), "no slug for miner {id}");
        }
        assert!(slug_for("99999").is_none());
    }

    #[test]
    fn the_two_new_miners_are_not_ones_the_normaliser_handles() {
        // lacre-meteo and oathcast-weather are live but absent from the
        // daemon-feed corpus, so the normaliser has no schema for them.
        // This test records that, so whoever adds one notices.
        for id in ["0", "64173"] {
            let slug = slug_for(id).expect("a slug");
            assert!(
                !schema::WEATHER_MINER_SLUGS.contains(&slug),
                "the normaliser now handles {slug}; update the report"
            );
        }
    }

    #[test]
    fn three_label_fields_name_a_container_not_a_scalar() {
        let containers: Vec<&str> = LABEL_FIELDS
            .iter()
            .filter(|(_, _, shape)| shape.contains("object") || shape.contains("array"))
            .map(|(id, _, _)| *id)
            .collect();
        assert_eq!(containers, vec!["211", "212", "0"]);
    }

    #[test]
    fn a_date_span_covers_the_whole_run() {
        let asks = vec![
            AskLine {
                index: 1,
                city_key: "tokyo".to_string(),
                query: "q".to_string(),
                asked_at_unix: hour_string_to_unix("2026-08-15T09:00"),
                status: 200,
                settled: true,
                miner_id: None,
                miner_name: None,
                intent: None,
                failure: None,
                body: String::new(),
            },
            AskLine {
                index: 2,
                city_key: "tokyo".to_string(),
                query: "q".to_string(),
                asked_at_unix: hour_string_to_unix("2026-08-16T02:00"),
                status: 200,
                settled: true,
                miner_id: None,
                miner_name: None,
                intent: None,
                failure: None,
                body: String::new(),
            },
        ];
        let (start, end) = date_span(&asks);
        assert_eq!(start, "2026-08-15");
        assert_eq!(end, "2026-08-16");
    }
}
