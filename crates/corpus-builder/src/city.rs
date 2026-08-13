//! City extraction from question text.
//!
//! This is deliberately simple: the feed's question text is templated,
//! not free text, so a small hardcoded table plus one coordinate pattern
//! covers every city that occurs in the corpus. This is used ONLY for
//! clustering (grouping paraphrases of the same question); it never feeds
//! the archive lookup, which always uses the miner's own reported
//! lat/lon.

/// One entry in the hardcoded city table: canonical lowercase name,
/// approximate centre latitude, approximate centre longitude (one decimal
/// place). Coordinates are only used to match the `latitude X, longitude
/// Y` pattern back to a name; they are not used for anything else.
const CITY_TABLE: &[(&str, f64, f64)] = &[
    ("dubai", 25.2, 55.3),
    ("riyadh", 24.7, 46.7),
    ("miami", 25.8, -80.2),
    ("gujranwala", 32.2, 74.2),
    ("tokyo", 35.7, 139.7),
    ("london", 51.5, -0.1),
    ("paris", 48.9, 2.3),
    ("rome", 41.9, 12.5),
    ("lisbon", 38.7, -9.1),
    ("berlin", 52.5, 13.4),
    ("madrid", 40.4, -3.7),
    ("lahore", 31.5, 74.3),
    ("vehari", 30.0, 72.3),
    ("chicago", 41.9, -87.6),
    ("maringá", -23.4, -52.0),
];

/// Max distance, in degrees on each axis, for a `latitude X, longitude Y`
/// pair to be matched back to a table city.
const COORD_TOLERANCE_DEG: f64 = 1.0;

/// Extract a canonical city name from templated question text.
///
/// Tries a direct substring match against [`CITY_TABLE`] first (case
/// insensitive), then falls back to parsing a `latitude X, longitude Y`
/// pattern and matching the parsed coordinate to the nearest table city
/// within [`COORD_TOLERANCE_DEG`]. Returns `None` when neither matches;
/// the caller counts these as unextractable.
pub fn extract_city(question_text: &str) -> Option<&'static str> {
    let lower = question_text.to_lowercase();

    for (name, _, _) in CITY_TABLE {
        if lower.contains(name) {
            return Some(name);
        }
    }

    if let Some((lat, lon)) = parse_lat_lon_pattern(&lower) {
        return nearest_city(lat, lon);
    }

    None
}

/// Parse a `latitude X, longitude Y` pattern out of lowercased text.
fn parse_lat_lon_pattern(lower: &str) -> Option<(f64, f64)> {
    let lat_idx = lower.find("latitude")?;
    let after_lat = &lower[lat_idx + "latitude".len()..];
    let lat = parse_leading_number(after_lat)?;

    let lon_idx = after_lat.find("longitude")?;
    let after_lon = &after_lat[lon_idx + "longitude".len()..];
    let lon = parse_leading_number(after_lon)?;

    Some((lat, lon))
}

/// Parse the first signed decimal number in `text`, skipping any leading
/// non-numeric characters (spaces, colons, commas).
fn parse_leading_number(text: &str) -> Option<f64> {
    let mut start = None;
    let mut end = None;
    for (i, c) in text.char_indices() {
        let is_num_char = c.is_ascii_digit() || c == '-' || c == '.';
        if is_num_char && start.is_none() {
            start = Some(i);
        }
        if start.is_some() {
            if is_num_char {
                end = Some(i + c.len_utf8());
            } else {
                break;
            }
        }
    }
    let (s, e) = (start?, end?);
    text[s..e].parse::<f64>().ok()
}

/// Find the table city nearest `(lat, lon)`, within tolerance.
fn nearest_city(lat: f64, lon: f64) -> Option<&'static str> {
    CITY_TABLE
        .iter()
        .find(|(_, clat, clon)| {
            (clat - lat).abs() <= COORD_TOLERANCE_DEG && (clon - lon).abs() <= COORD_TOLERANCE_DEG
        })
        .map(|(name, _, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_direct_city_name_case_insensitively() {
        assert_eq!(extract_city("What is the weather in Paris?"), Some("paris"));
        assert_eq!(
            extract_city("Weather in gujranwala for next week?"),
            Some("gujranwala")
        );
    }

    #[test]
    fn matches_accented_city_name() {
        assert_eq!(
            extract_city("how will be the weather in Maringá PR Brazil this weekend"),
            Some("maringá")
        );
    }

    #[test]
    fn matches_lat_lon_pattern_to_dubai() {
        assert_eq!(
            extract_city("What's the weather forecast for latitude 25.2, longitude 55.3?"),
            Some("dubai")
        );
    }

    #[test]
    fn matches_lat_lon_pattern_with_negative_longitude() {
        assert_eq!(
            extract_city("forecast for latitude 25.8, longitude -80.2 please"),
            Some("miami")
        );
    }

    #[test]
    fn no_city_found_returns_none() {
        assert_eq!(extract_city("[direct] 18 -> /predict"), None);
        assert_eq!(
            extract_city("Will upper stage impact the moon on August 5?"),
            None
        );
    }

    #[test]
    fn lat_lon_pattern_outside_tolerance_returns_none() {
        assert_eq!(
            extract_city("forecast for latitude 0.0, longitude 0.0"),
            None
        );
    }
}
