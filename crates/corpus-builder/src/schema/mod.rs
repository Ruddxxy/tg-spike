//! Per-miner schema normalisers.
//!
//! Each submodule knows the exact field paths for one miner's response
//! shape and converts it to a flat list of (valid time, Celsius) points.
//! Nothing here does scoring; it only extracts and unit-converts.

mod openweathermap;
mod weatherapi;
mod zeus;

use serde_json::Value;

/// One (valid time, Celsius temperature, lat, lon) point taken from a
/// single miner response. A response can produce many points (Zeus
/// responses hold about 72 hours each).
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherPoint {
    /// Naive minute-precision UTC, e.g. `"2026-08-10T12:00"`. Matches the
    /// Open-Meteo archive's own time format so the two can be joined as
    /// plain strings, with no timezone conversion at the join site.
    pub valid_time_utc: String,
    /// The miner's own claimed temperature, in Celsius, NOT rounded. This
    /// is compared against the (rounded) archive actual for error stats.
    /// Rounding only ever happens in `rounding::round_temp_c`, and only
    /// for ground-truth renderings.
    pub temp_c: f64,
    pub lat: f64,
    pub lon: f64,
}

/// Why a response was excluded from the usable corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropReason {
    /// `execution.result` was JSON null.
    NullResult,
    /// The miner slug is not one of the three weather miners this crate
    /// handles.
    NotAWeatherMiner,
    /// A declared unit field was absent.
    MissingUnit,
    /// A declared unit field held an unexpected value.
    WrongUnit,
    /// Two parallel arrays that should be the same length were not.
    LengthMismatch,
    /// A converted value fell outside any physically plausible range, so
    /// it was dropped instead of guessed.
    ImplausibleValue,
    /// The response did not match the expected JSON shape at all.
    UnrecognisedShape,
}

impl DropReason {
    /// Short machine-stable label, used as a report key.
    pub fn label(self) -> &'static str {
        match self {
            DropReason::NullResult => "null_result",
            DropReason::NotAWeatherMiner => "not_a_weather_miner",
            DropReason::MissingUnit => "missing_unit",
            DropReason::WrongUnit => "wrong_unit",
            DropReason::LengthMismatch => "length_mismatch",
            DropReason::ImplausibleValue => "implausible_value",
            DropReason::UnrecognisedShape => "unrecognised_shape",
        }
    }
}

/// Normalise one miner response, given its slug and raw `result` value.
///
/// Returns every (valid time, Celsius) point the response contains, or
/// the reason the whole response was dropped. Null results are checked
/// here, before dispatch, regardless of the feed's reported `status`: the
/// feed has null results tagged both `"success"` and `"error"` in
/// practice, so status is not a reliable signal on its own.
pub fn normalise(miner_slug: &str, result: &Value) -> Result<Vec<WeatherPoint>, DropReason> {
    if result.is_null() {
        return Err(DropReason::NullResult);
    }
    match miner_slug {
        "weatherapi" => weatherapi::normalise(result),
        "openweathermap" => openweathermap::normalise(result),
        "bittensor-sn18-zeus" => zeus::normalise(result),
        _ => Err(DropReason::NotAWeatherMiner),
    }
}

/// The three miner slugs this crate treats as weather sources.
pub const WEATHER_MINER_SLUGS: [&str; 3] = ["weatherapi", "openweathermap", "bittensor-sn18-zeus"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_result_is_flagged_before_dispatch() {
        let result = Value::Null;
        assert_eq!(
            normalise("weatherapi", &result),
            Err(DropReason::NullResult)
        );
        assert_eq!(
            normalise("openweathermap", &result),
            Err(DropReason::NullResult)
        );
        assert_eq!(
            normalise("bittensor-sn18-zeus", &result),
            Err(DropReason::NullResult)
        );
    }

    #[test]
    fn unknown_miner_slug_is_rejected() {
        let result = serde_json::json!({"anything": 1});
        assert_eq!(
            normalise("alphavantage", &result),
            Err(DropReason::NotAWeatherMiner)
        );
    }
}
