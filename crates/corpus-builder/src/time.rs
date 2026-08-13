//! Timestamp conversion helpers.
//!
//! Every valid time in this crate ends up as naive minute-precision UTC,
//! e.g. `"2026-08-10T12:00"` (no seconds, no trailing `Z`). This is the
//! Open-Meteo archive's own time format, so joining a miner's valid time
//! against an archive actual is a plain string comparison, with no
//! timezone maths at the lookup site.

use chrono::{DateTime, Utc};

/// Convert a Unix epoch-seconds timestamp to naive minute-precision UTC.
///
/// Returns `None` if `epoch` is outside the range `chrono` can represent.
/// A caller sees this as "could not extract a valid time" and skips the
/// point rather than inventing one.
pub fn epoch_to_naive_minute(epoch: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(epoch, 0).map(|dt| dt.format("%Y-%m-%dT%H:%M").to_string())
}

/// Parse an ISO 8601 UTC timestamp with seconds and a `Z` suffix (Zeus's
/// `hourly.time` format, e.g. `"2026-08-10T12:00:00Z"`) into naive
/// minute-precision UTC.
///
/// Returns `None` if the string does not parse as RFC 3339.
pub fn iso_to_naive_minute(s: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).format("%Y-%m-%dT%H:%M").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_converts_to_naive_minute() {
        // This is the `last_updated_epoch` from the WeatherAPI Paris
        // fixture used elsewhere in this crate's tests. Its UTC instant
        // is 12:30, NOT the 14:30 shown in that same response's
        // `last_updated` string: that string is Paris LOCAL time
        // (UTC+2 in August), which this crate never reads. Only the
        // epoch field is authoritative for UTC.
        assert_eq!(
            epoch_to_naive_minute(1_786_537_800),
            Some("2026-08-12T12:30".to_string())
        );
    }

    #[test]
    fn iso_with_seconds_and_z_truncates_to_minute() {
        assert_eq!(
            iso_to_naive_minute("2026-08-10T12:00:00Z"),
            Some("2026-08-10T12:00".to_string())
        );
    }

    #[test]
    fn garbage_iso_string_returns_none() {
        assert_eq!(iso_to_naive_minute("not a timestamp"), None);
    }
}
