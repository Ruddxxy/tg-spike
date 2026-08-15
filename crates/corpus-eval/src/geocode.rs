//! This module resolves a city name to coordinates, ONCE, and caches
//! the answer.
//!
//! ## Why this exists at all
//!
//! Wave 2 joined the archive at the coordinates the MINER returned. A
//! miner that answered for Brazil, Indiana was scored against Indiana's
//! weather, so the pair was self-consistent and the error was
//! undetectable. Section 2.4 of the evaluation reports that zero of
//! five known-bad rows were caught for exactly that reason.
//!
//! This module is the fix. The coordinates come from the city list the
//! batch plan holds, resolved by Open-Meteo's geocoder. A miner
//! response is never read here, and the file this writes is the only
//! source of coordinates the head-to-head join uses.
//!
//! ## Why the country is part of the query
//!
//! "Maringá" resolves to Indiana. Every lookup sends the country and
//! then CHECKS the country that came back, because a geocoder that
//! silently picks another continent is the failure being designed out.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The Open-Meteo geocoding endpoint. Free and needs no key.
const GEOCODE_BASE: &str = "https://geocoding-api.open-meteo.com/v1/search";

/// Where the resolved coordinates are cached.
pub const COORDS_PATH: &str = "corpus/city-coords.json";

/// One resolved city.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coordinates {
    /// The city key from the batch plan.
    pub key: String,
    /// The name that was searched.
    pub name: String,
    /// The country that was asked for.
    pub wanted_country: String,
    /// The country the geocoder returned.
    pub found_country: String,
    /// The resolved latitude.
    pub latitude: f64,
    /// The resolved longitude.
    pub longitude: f64,
    /// The time zone the geocoder reports, for the report only.
    #[serde(default)]
    pub timezone: String,
}

/// One city in the batch plan, as the plan file stores it.
#[derive(Debug, Deserialize)]
pub struct PlannedCity {
    /// The stable key.
    pub key: String,
    /// The city name.
    pub name: String,
    /// The country.
    pub country: String,
    /// The exact query string the batch sent.
    ///
    /// Kept so the plan file stays a complete record of what was
    /// asked. The geocoder resolves from the name and country.
    #[allow(dead_code)]
    pub query: String,
}

/// The batch plan file.
#[derive(Debug, Deserialize)]
pub struct Plan {
    /// Every city the batch asked about.
    pub cities: Vec<PlannedCity>,
}

/// This function reads the batch plan.
pub fn load_plan(path: &Path) -> Result<Plan, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read the batch plan {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("the batch plan is not valid JSON: {error}"))
}

/// One result row from the geocoder.
#[derive(Debug, Deserialize)]
struct GeocodeHit {
    name: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    country: String,
    #[serde(default)]
    timezone: String,
}

/// The geocoder response.
#[derive(Debug, Deserialize)]
struct GeocodeResponse {
    #[serde(default)]
    results: Vec<GeocodeHit>,
}

/// This function resolves one city.
///
/// The function returns an error when the geocoder gives nothing, or
/// when every hit names a country other than the one asked for. It
/// never falls back to the first hit regardless of country, because
/// that is the Maringá defect.
fn resolve(name: &str, country: &str) -> Result<(f64, f64, String, String), String> {
    let query = format!(
        "{GEOCODE_BASE}?name={}&count=10&language=en&format=json",
        urlencode(name)
    );
    let body = ureq::get(&query)
        .call()
        .map_err(|error| format!("the geocoder request for {name:?} failed: {error}"))?
        .into_string()
        .map_err(|error| format!("cannot read the geocoder response for {name:?}: {error}"))?;

    let parsed: GeocodeResponse = serde_json::from_str(&body)
        .map_err(|error| format!("the geocoder response for {name:?} is not JSON: {error}"))?;

    if parsed.results.is_empty() {
        return Err(format!("the geocoder found nothing for {name:?}"));
    }

    // The country must match. A hit on the right name in the wrong
    // country is the exact failure this module exists to prevent.
    let wanted = country.to_lowercase();
    for hit in &parsed.results {
        if hit.country.to_lowercase() == wanted {
            return Ok((
                hit.latitude,
                hit.longitude,
                hit.country.clone(),
                hit.timezone.clone(),
            ));
        }
    }

    let offered: Vec<String> = parsed
        .results
        .iter()
        .map(|hit| format!("{} in {}", hit.name, hit.country))
        .collect();
    Err(format!(
        "the geocoder found {name:?} but never in {country:?}. It offered: {}",
        offered.join("; ")
    ))
}

/// This function percent-encodes a query term.
///
/// Written out rather than pulled from a crate, because the only
/// characters a city name needs are spaces and non-ASCII letters.
fn urlencode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// This function resolves every city in the plan and caches the result.
///
/// A city already in the cache is NOT looked up again. The instruction
/// is to geocode once, and a rerun that re-queried would also risk a
/// different answer for the same key.
pub fn resolve_plan(plan: &Plan, path: &Path) -> Result<BTreeMap<String, Coordinates>, String> {
    let mut cached: BTreeMap<String, Coordinates> = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => BTreeMap::new(),
    };

    let mut failures = Vec::new();
    for city in &plan.cities {
        if cached.contains_key(&city.key) {
            println!("  {:<12} cached", city.key);
            continue;
        }
        match resolve(&city.name, &city.country) {
            Ok((latitude, longitude, found_country, timezone)) => {
                println!(
                    "  {:<12} {latitude:>9.4} {longitude:>10.4}  {found_country} ({timezone})",
                    city.key
                );
                cached.insert(
                    city.key.clone(),
                    Coordinates {
                        key: city.key.clone(),
                        name: city.name.clone(),
                        wanted_country: city.country.clone(),
                        found_country,
                        latitude,
                        longitude,
                        timezone,
                    },
                );
            }
            Err(error) => {
                println!("  {:<12} FAILED: {error}", city.key);
                failures.push(error);
            }
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot make the coordinate directory: {error}"))?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&cached).unwrap_or_default(),
    )
    .map_err(|error| format!("cannot write the coordinates: {error}"))?;

    if !failures.is_empty() {
        return Err(format!(
            "{} of {} cities did not resolve; the batch must not run until they do",
            failures.len(),
            plan.cities.len()
        ));
    }
    Ok(cached)
}

/// This function reads the cached coordinates.
pub fn load_coordinates(path: &Path) -> Result<BTreeMap<String, Coordinates>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "cannot read {}: {error}. Run `geocode` first.",
            path.display()
        )
    })?;
    serde_json::from_str(&text)
        .map_err(|error| format!("the coordinate cache is not valid JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_space_and_an_accent_are_encoded() {
        assert_eq!(urlencode("New York"), "New%20York");
        assert_eq!(urlencode("Maringa"), "Maringa");
        // A non-ASCII letter must survive as percent-encoded UTF-8.
        assert!(urlencode("Maringá").starts_with("Maring"));
        assert!(urlencode("Maringá").contains('%'));
    }

    #[test]
    fn plain_characters_are_left_alone() {
        assert_eq!(urlencode("Tokyo"), "Tokyo");
        assert_eq!(urlencode("abc-123_x.y~z"), "abc-123_x.y~z");
    }

    #[test]
    fn a_plan_parses() {
        let text = r#"{"cities":[{"key":"tokyo","name":"Tokyo","country":"Japan",
                       "query":"What is the current weather in Tokyo, Japan?"}]}"#;
        let plan: Plan = serde_json::from_str(text).expect("the plan must parse");
        assert_eq!(plan.cities.len(), 1);
        assert_eq!(plan.cities[0].key, "tokyo");
    }
}
