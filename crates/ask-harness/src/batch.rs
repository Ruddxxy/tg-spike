//! This module holds the head-to-head batch plan.
//!
//! ## Design for spread, not depth
//!
//! The probe showed a fixed query reaching 2 miners in a 23/7 split. A
//! pair needs 2 or more miners on the SAME query string, so the useful
//! quantity is not how many asks are sent but how many query strings
//! collect more than one miner.
//!
//! Ten query strings with about twenty asks each beats one query string
//! with two hundred asks. At a 23 percent minority rate, twenty asks
//! miss the minority miner only about 0.5 percent of the time, so ten
//! cities should nearly all pair. Two hundred asks of ONE city would
//! give one pair and a very precise split for a single question.
//!
//! ## Why the city list is fixed here
//!
//! The city is the unit of ground truth. Its coordinates are geocoded
//! ONCE from this list and never read out of a miner response, because
//! wave 2 joined truth at miner-returned coordinates and that is
//! exactly what made its five known-bad rows undetectable.
//!
//! The list spans time zones so the asks land at different local hours,
//! which keeps the set from being ten samples of the same weather.

/// One city in the batch plan.
pub struct City {
    /// The stable key used as the cluster id and the geocoding term.
    pub key: &'static str,
    /// The name as it goes into the query text.
    pub name: &'static str,
    /// The country, which the geocoder needs to disambiguate.
    ///
    /// Wave 2 resolved "Maringá PR Brazil" to Brazil, Indiana. A bare
    /// city name is not a location.
    pub country: &'static str,
    /// Roughly how far the city sits from UTC, for the report only.
    pub utc_offset_hours: i8,
}

/// The fixed city list. Ten cities, spread across the day.
pub const CITIES: [City; 10] = [
    City {
        key: "auckland",
        name: "Auckland",
        country: "New Zealand",
        utc_offset_hours: 12,
    },
    City {
        key: "tokyo",
        name: "Tokyo",
        country: "Japan",
        utc_offset_hours: 9,
    },
    City {
        key: "singapore",
        name: "Singapore",
        country: "Singapore",
        utc_offset_hours: 8,
    },
    City {
        key: "dubai",
        name: "Dubai",
        country: "United Arab Emirates",
        utc_offset_hours: 4,
    },
    City {
        key: "nairobi",
        name: "Nairobi",
        country: "Kenya",
        utc_offset_hours: 3,
    },
    City {
        key: "berlin",
        name: "Berlin",
        country: "Germany",
        utc_offset_hours: 2,
    },
    City {
        key: "lagos",
        name: "Lagos",
        country: "Nigeria",
        utc_offset_hours: 1,
    },
    City {
        key: "reykjavik",
        name: "Reykjavik",
        country: "Iceland",
        utc_offset_hours: 0,
    },
    City {
        key: "new_york",
        name: "New York",
        country: "United States",
        utc_offset_hours: -4,
    },
    City {
        key: "denver",
        name: "Denver",
        country: "United States",
        utc_offset_hours: -6,
    },
];

/// How many asks each city gets.
pub const ASKS_PER_CITY: usize = 20;

impl City {
    /// This function gives the exact query string for a city.
    ///
    /// Every ask for a city sends this string unchanged. A pair is
    /// defined as 2 or more miners answering the SAME string, so the
    /// string must not vary within a city.
    pub fn query(&self) -> String {
        format!(
            "What is the current weather in {}, {}?",
            self.name, self.country
        )
    }
}

/// This function gives the total ask count of the plan.
pub fn total_asks() -> usize {
    CITIES.len() * ASKS_PER_CITY
}

/// The chance that a miner holding `share` of routing is missed
/// entirely across `asks` independent asks.
///
/// This is what the plan is sized against. It is reported so the pair
/// yield can be checked against a stated expectation rather than a
/// feeling.
pub fn miss_probability(share: f64, asks: usize) -> f64 {
    (1.0 - share).powi(asks as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_is_ten_cities_and_two_hundred_asks() {
        assert_eq!(CITIES.len(), 10);
        assert_eq!(total_asks(), 200);
    }

    #[test]
    fn every_city_key_is_unique_and_lowercase() {
        for (index, city) in CITIES.iter().enumerate() {
            assert_eq!(
                city.key,
                city.key.to_lowercase(),
                "{} is not lowercase",
                city.key
            );
            for other in CITIES.iter().skip(index + 1) {
                assert_ne!(city.key, other.key, "the key {} repeats", city.key);
            }
        }
    }

    #[test]
    fn the_cities_span_the_day() {
        // Ten cities in one time zone would be ten samples of the same
        // hour. The spread is the point.
        let mut offsets: Vec<i8> = CITIES.iter().map(|c| c.utc_offset_hours).collect();
        offsets.sort_unstable();
        let span = offsets[offsets.len() - 1] - offsets[0];
        assert!(span >= 16, "the cities span only {span} hours");
    }

    #[test]
    fn a_query_names_the_country_so_the_geocoder_cannot_pick_the_wrong_continent() {
        let city = &CITIES[0];
        let query = city.query();
        assert!(query.contains("Auckland"));
        assert!(query.contains("New Zealand"));
    }

    #[test]
    fn twenty_asks_at_the_observed_minority_share_almost_never_miss() {
        // The probe saw a 23/7 split, so the minority miner holds about
        // 0.233. This is the number the pair yield is judged against.
        let miss = miss_probability(7.0 / 30.0, ASKS_PER_CITY);
        assert!(
            miss < 0.01,
            "a city would miss the minority miner {miss} of the time"
        );
    }
}
