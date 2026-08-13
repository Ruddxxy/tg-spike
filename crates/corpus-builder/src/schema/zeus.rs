//! Normaliser for Bittensor SN18 (Zeus) responses.

use serde_json::Value;

use super::{DropReason, WeatherPoint};
use crate::time::iso_to_naive_minute;

/// Extract every (valid time, Celsius) point from a Zeus response.
///
/// Zeus is the only one of the three schemas that declares its own unit
/// (`hourly_units."2t"`). This function asserts that declaration instead
/// of assuming Kelvin: if the key is missing, is not exactly `"K"`, or
/// the `2t` and `time` arrays disagree in length, the whole response is
/// dropped rather than guessed. Zeus also carries its own top-level
/// `latitude` / `longitude`, so it needs no geocoding.
pub fn normalise(result: &Value) -> Result<Vec<WeatherPoint>, DropReason> {
    let lat = result
        .get("latitude")
        .and_then(Value::as_f64)
        .ok_or(DropReason::UnrecognisedShape)?;
    let lon = result
        .get("longitude")
        .and_then(Value::as_f64)
        .ok_or(DropReason::UnrecognisedShape)?;

    let unit = result
        .get("hourly_units")
        .and_then(|u| u.get("2t"))
        .and_then(Value::as_str);
    match unit {
        Some("K") => {}
        Some(_) => return Err(DropReason::WrongUnit),
        None => return Err(DropReason::MissingUnit),
    }

    let hourly = result.get("hourly").ok_or(DropReason::UnrecognisedShape)?;
    let temps = hourly
        .get("2t")
        .and_then(Value::as_array)
        .ok_or(DropReason::UnrecognisedShape)?;
    let times = hourly
        .get("time")
        .and_then(Value::as_array)
        .ok_or(DropReason::UnrecognisedShape)?;

    if temps.len() != times.len() {
        return Err(DropReason::LengthMismatch);
    }

    let mut points = Vec::with_capacity(temps.len());
    for (t, time_val) in temps.iter().zip(times.iter()) {
        let (Some(kelvin), Some(time_str)) = (t.as_f64(), time_val.as_str()) else {
            continue;
        };
        let Some(valid_time_utc) = iso_to_naive_minute(time_str) else {
            continue;
        };
        points.push(WeatherPoint {
            valid_time_utc,
            temp_c: kelvin - 273.15,
            lat,
            lon,
        });
    }

    if points.is_empty() {
        return Err(DropReason::UnrecognisedShape);
    }

    Ok(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> serde_json::Value {
        serde_json::json!({
            "latitude": 25.75,
            "longitude": -80.25,
            "hourly_units": {"2t": "K", "time": "iso8601"},
            "hourly": {
                "2t": [302.25, 302.75],
                "time": ["2026-08-10T12:00:00Z", "2026-08-10T13:00:00Z"]
            }
        })
    }

    #[test]
    fn extracts_all_hours_and_converts_kelvin() {
        let points = normalise(&base()).expect("should normalise");
        assert_eq!(points.len(), 2);
        assert!((points[0].temp_c - (302.25 - 273.15)).abs() < 1e-9);
        assert_eq!(points[0].valid_time_utc, "2026-08-10T12:00");
        assert_eq!(points[0].lat, 25.75);
        assert_eq!(points[0].lon, -80.25);
    }

    #[test]
    fn missing_unit_key_is_flagged_and_excluded() {
        let mut v = base();
        v["hourly_units"]
            .as_object_mut()
            .expect("object")
            .remove("2t");
        assert_eq!(normalise(&v), Err(DropReason::MissingUnit));
    }

    #[test]
    fn wrong_unit_value_is_flagged_and_excluded() {
        let mut v = base();
        v["hourly_units"]["2t"] = serde_json::json!("C");
        assert_eq!(normalise(&v), Err(DropReason::WrongUnit));
    }

    #[test]
    fn mismatched_array_lengths_are_flagged_and_excluded() {
        let mut v = base();
        v["hourly"]["2t"] = serde_json::json!([302.25, 302.75, 303.0]);
        assert_eq!(normalise(&v), Err(DropReason::LengthMismatch));
    }

    #[test]
    fn missing_lat_lon_is_rejected() {
        let mut v = base();
        v.as_object_mut().expect("object").remove("latitude");
        assert_eq!(normalise(&v), Err(DropReason::UnrecognisedShape));
    }
}
