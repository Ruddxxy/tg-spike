//! Normaliser for WeatherAPI responses (`current` + optional `forecast`).

use serde_json::Value;

use super::{DropReason, WeatherPoint};
use crate::time::epoch_to_naive_minute;

/// Extract every (valid time, Celsius) point from a WeatherAPI response.
///
/// WeatherAPI already reports Celsius directly (the `temp_c` fields), so
/// there is no unit conversion and nothing to assert: the field name
/// states its own unit. Only `location.lat` / `location.lon` and the
/// `_epoch` valid times are used; the human-readable local time strings
/// (`last_updated`, forecast `hour[].time`) are ignored because they are
/// local time, not UTC, and the epoch fields already give an unambiguous
/// UTC instant.
pub fn normalise(result: &Value) -> Result<Vec<WeatherPoint>, DropReason> {
    let location = result
        .get("location")
        .ok_or(DropReason::UnrecognisedShape)?;
    let lat = location
        .get("lat")
        .and_then(Value::as_f64)
        .ok_or(DropReason::UnrecognisedShape)?;
    let lon = location
        .get("lon")
        .and_then(Value::as_f64)
        .ok_or(DropReason::UnrecognisedShape)?;

    let mut points = Vec::new();

    if let Some(current) = result.get("current") {
        if let (Some(temp_c), Some(epoch)) = (
            current.get("temp_c").and_then(Value::as_f64),
            current.get("last_updated_epoch").and_then(Value::as_i64),
        ) {
            if let Some(valid_time_utc) = epoch_to_naive_minute(epoch) {
                points.push(WeatherPoint {
                    valid_time_utc,
                    temp_c,
                    lat,
                    lon,
                });
            }
        }
    }

    if let Some(days) = result
        .get("forecast")
        .and_then(|f| f.get("forecastday"))
        .and_then(Value::as_array)
    {
        for day in days {
            let Some(hours) = day.get("hour").and_then(Value::as_array) else {
                continue;
            };
            for hour in hours {
                if let (Some(temp_c), Some(epoch)) = (
                    hour.get("temp_c").and_then(Value::as_f64),
                    hour.get("time_epoch").and_then(Value::as_i64),
                ) {
                    if let Some(valid_time_utc) = epoch_to_naive_minute(epoch) {
                        points.push(WeatherPoint {
                            valid_time_utc,
                            temp_c,
                            lat,
                            lon,
                        });
                    }
                }
            }
        }
    }

    if points.is_empty() {
        return Err(DropReason::UnrecognisedShape);
    }

    Ok(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_current_only() {
        let result = serde_json::json!({
            "current": {"temp_c": 34.7, "last_updated_epoch": 1_786_537_800},
            "location": {"lat": 48.8667, "lon": 2.3333}
        });
        let points = normalise(&result).expect("should normalise");
        assert_eq!(points.len(), 1);
        // 1_786_537_800 is 2026-08-12T12:30:00Z. The response's own
        // `last_updated` string would show 14:30 (Paris local time,
        // UTC+2), but this crate only reads the epoch field.
        assert_eq!(points[0].valid_time_utc, "2026-08-12T12:30");
        assert_eq!(points[0].temp_c, 34.7);
        assert_eq!(points[0].lat, 48.8667);
        assert_eq!(points[0].lon, 2.3333);
    }

    #[test]
    fn extracts_current_and_forecast_hours() {
        let result = serde_json::json!({
            "current": {"temp_c": 21.4, "last_updated_epoch": 1_786_077_900},
            "location": {"lat": -23.4, "lon": -52.0},
            "forecast": {"forecastday": [
                {"hour": [
                    {"temp_c": 20.0, "time_epoch": 1_786_060_800},
                    {"temp_c": 22.5, "time_epoch": 1_786_064_400}
                ]}
            ]}
        });
        let points = normalise(&result).expect("should normalise");
        assert_eq!(points.len(), 3);
    }

    #[test]
    fn missing_location_is_rejected() {
        let result = serde_json::json!({"current": {"temp_c": 20.0, "last_updated_epoch": 1}});
        assert_eq!(normalise(&result), Err(DropReason::UnrecognisedShape));
    }

    #[test]
    fn missing_current_and_forecast_is_rejected() {
        let result = serde_json::json!({"location": {"lat": 1.0, "lon": 1.0}});
        assert_eq!(normalise(&result), Err(DropReason::UnrecognisedShape));
    }
}
