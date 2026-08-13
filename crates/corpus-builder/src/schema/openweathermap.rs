//! Normaliser for OpenWeatherMap responses: 5-day/3-hour forecast
//! (`list`) or a single-shot `/weather` response (top-level `main` +
//! `dt`, `coord` instead of `city.coord`).

use serde_json::Value;

use super::{DropReason, WeatherPoint};
use crate::time::epoch_to_naive_minute;

/// OpenWeatherMap never puts a unit key in its response body (unlike Zeus
/// and Open-Meteo, which both declare their unit explicitly). Its
/// documented default is Kelvin, but live data in this corpus shows that
/// is not reliable: two responses (both `name: "London"`, same station
/// id) return `main.temp` already in Celsius (~32-33), not Kelvin
/// (~305), presumably because that particular call passed
/// `units=metric`. There is no field in the response that says which
/// happened.
///
/// So the unit is inferred from magnitude instead of assumed:
///   - raw >= KELVIN_THRESHOLD  -> Kelvin, convert by subtracting 273.15.
///   - raw <  KELVIN_THRESHOLD  -> already Celsius, no conversion.
///
/// No weather station on Earth reports 100 C or higher, and no Kelvin
/// reading for surface air temperature is ever below 100 K, so the two
/// interpretations never collide at this threshold.
const KELVIN_THRESHOLD: f64 = 100.0;

/// After inferring the unit, the resulting Celsius value must land in
/// this range or the point is implausible and is dropped, not guessed.
const PLAUSIBLE_MIN_C: f64 = -90.0;
const PLAUSIBLE_MAX_C: f64 = 60.0;

fn to_celsius(raw: f64) -> Option<f64> {
    let celsius = if raw >= KELVIN_THRESHOLD {
        raw - 273.15
    } else {
        raw
    };
    if (PLAUSIBLE_MIN_C..=PLAUSIBLE_MAX_C).contains(&celsius) {
        Some(celsius)
    } else {
        None
    }
}

/// Extract every (valid time, Celsius) point from an OpenWeatherMap
/// response, handling both the `list` (forecast) and single-shot shapes.
pub fn normalise(result: &Value) -> Result<Vec<WeatherPoint>, DropReason> {
    if let Some(list) = result.get("list").and_then(Value::as_array) {
        return normalise_list(result, list);
    }
    if let Some(main) = result.get("main") {
        return normalise_single(result, main);
    }
    Err(DropReason::UnrecognisedShape)
}

fn normalise_list(result: &Value, list: &[Value]) -> Result<Vec<WeatherPoint>, DropReason> {
    let coord = result
        .get("city")
        .and_then(|c| c.get("coord"))
        .ok_or(DropReason::UnrecognisedShape)?;
    let lat = coord
        .get("lat")
        .and_then(Value::as_f64)
        .ok_or(DropReason::UnrecognisedShape)?;
    let lon = coord
        .get("lon")
        .and_then(Value::as_f64)
        .ok_or(DropReason::UnrecognisedShape)?;

    let mut points = Vec::new();
    for item in list {
        let (Some(raw), Some(dt)) = (
            item.get("main")
                .and_then(|m| m.get("temp"))
                .and_then(Value::as_f64),
            item.get("dt").and_then(Value::as_i64),
        ) else {
            continue;
        };
        let Some(temp_c) = to_celsius(raw) else {
            continue;
        };
        let Some(valid_time_utc) = epoch_to_naive_minute(dt) else {
            continue;
        };
        points.push(WeatherPoint {
            valid_time_utc,
            temp_c,
            lat,
            lon,
        });
    }

    if points.is_empty() {
        return Err(DropReason::ImplausibleValue);
    }
    Ok(points)
}

fn normalise_single(result: &Value, main: &Value) -> Result<Vec<WeatherPoint>, DropReason> {
    let coord = result.get("coord").ok_or(DropReason::UnrecognisedShape)?;
    let lat = coord
        .get("lat")
        .and_then(Value::as_f64)
        .ok_or(DropReason::UnrecognisedShape)?;
    let lon = coord
        .get("lon")
        .and_then(Value::as_f64)
        .ok_or(DropReason::UnrecognisedShape)?;
    let raw = main
        .get("temp")
        .and_then(Value::as_f64)
        .ok_or(DropReason::UnrecognisedShape)?;
    let dt = result
        .get("dt")
        .and_then(Value::as_i64)
        .ok_or(DropReason::UnrecognisedShape)?;

    let temp_c = to_celsius(raw).ok_or(DropReason::ImplausibleValue)?;
    let valid_time_utc = epoch_to_naive_minute(dt).ok_or(DropReason::UnrecognisedShape)?;

    Ok(vec![WeatherPoint {
        valid_time_utc,
        temp_c,
        lat,
        lon,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_shot_kelvin_converts() {
        let result = serde_json::json!({
            "coord": {"lat": 25.2582, "lon": 55.3047},
            "main": {"temp": 306.17},
            "dt": 1_785_464_233
        });
        let points = normalise(&result).expect("should normalise");
        assert_eq!(points.len(), 1);
        assert!((points[0].temp_c - 33.02).abs() < 0.01);
    }

    #[test]
    fn single_shot_already_celsius_is_not_double_converted() {
        // Real anomaly seen in the live feed: a London /weather response
        // with temp already in Celsius, not Kelvin.
        let result = serde_json::json!({
            "coord": {"lat": 51.5085, "lon": -0.1257},
            "main": {"temp": 32.39},
            "dt": 1_785_346_002
        });
        let points = normalise(&result).expect("should normalise");
        assert_eq!(points[0].temp_c, 32.39);
    }

    #[test]
    fn list_shape_converts_every_entry() {
        let result = serde_json::json!({
            "city": {"coord": {"lat": 24.6877, "lon": 46.7219}},
            "list": [
                {"dt": 1_785_639_600, "main": {"temp": 305.98}},
                {"dt": 1_785_650_400, "main": {"temp": 310.0}}
            ]
        });
        let points = normalise(&result).expect("should normalise");
        assert_eq!(points.len(), 2);
    }

    #[test]
    fn implausible_value_is_dropped_not_guessed() {
        // 150 is >= threshold so treated as Kelvin -> -123.15 C, outside
        // the plausible range. Must be dropped, not emitted.
        let result = serde_json::json!({
            "coord": {"lat": 1.0, "lon": 1.0},
            "main": {"temp": 150.0},
            "dt": 1_785_639_600
        });
        assert_eq!(normalise(&result), Err(DropReason::ImplausibleValue));
    }

    #[test]
    fn missing_coord_is_rejected() {
        let result = serde_json::json!({"main": {"temp": 300.0}, "dt": 1});
        assert_eq!(normalise(&result), Err(DropReason::UnrecognisedShape));
    }

    #[test]
    fn neither_list_nor_main_is_rejected() {
        let result = serde_json::json!({"message": "rate limited"});
        assert_eq!(normalise(&result), Err(DropReason::UnrecognisedShape));
    }
}
