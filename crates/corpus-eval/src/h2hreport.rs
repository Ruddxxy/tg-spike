//! This module prints the head-to-head report.
//!
//! A "pair" here has one meaning and it is strict: two or more DISTINCT
//! miners answered the SAME query string. The query string is fixed per
//! city, so a city pairs when its asks reached more than one miner.
//!
//! A city that reached one miner twenty times is not a pair. It is
//! twenty samples of one miner, and it can rank nothing.

use std::collections::{BTreeMap, BTreeSet};

use crate::headtohead::{label_field_for, DropCounts, JoinedRow};

/// The per-miner accuracy summary.
pub struct MinerAccuracy {
    /// The miner id.
    pub miner_id: String,
    /// The slug the normaliser used.
    pub miner_slug: String,
    /// How many rows this miner has.
    pub rows: usize,
    /// The mean absolute error against the archive, in Celsius.
    pub mean_absolute_error: f64,
    /// The median absolute error, in Celsius.
    pub median_absolute_error: f64,
    /// The mean signed error, which shows a warm or cold bias.
    pub mean_signed_error: f64,
    /// The largest absolute error seen.
    pub worst_absolute_error: f64,
    /// The mean gap between the miner's claimed observation time and
    /// the ask hour, in minutes.
    pub mean_drift_minutes: f64,
}

/// This function prints every part of the report.
///
/// The function returns whether the pair yield met what the probe's
/// split predicted.
pub fn print_report(
    rows: &[JoinedRow],
    asks_per_city: usize,
    minority_share: f64,
    drops: &DropCounts,
    settled_units: u64,
    observed_miner_ids: &BTreeSet<String>,
) -> bool {
    println!();
    println!("=== PAIRS PER CITY ===");
    println!("a pair means 2+ DISTINCT miners answered the SAME query string");
    println!();

    // City -> miner id -> row count.
    let mut by_city: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for row in rows {
        *by_city
            .entry(row.city_key.clone())
            .or_default()
            .entry(row.miner_id.clone())
            .or_insert(0) += 1;
    }

    println!(
        "{:<12} {:>5} {:>8}  split by miner id",
        "city", "rows", "miners"
    );
    let mut paired_cities = 0usize;
    for (city, miners) in &by_city {
        let split: Vec<String> = miners
            .iter()
            .map(|(id, count)| format!("{id}x{count}"))
            .collect();
        let rows_here: usize = miners.values().sum();
        if miners.len() > 1 {
            paired_cities += 1;
        }
        println!(
            "{city:<12} {rows_here:>5} {:>8}  {}",
            miners.len(),
            split.join(" ")
        );
    }

    let cities = by_city.len();
    println!();
    println!("cities with rows:   {cities}");
    println!("cities that paired: {paired_cities}");

    println!();
    println!("=== MINERS OBSERVED ===");
    println!(
        "{:<10} {:<24} {:<22} in the joined set?",
        "miner_id", "label_field", "shape"
    );
    for id in observed_miner_ids {
        let (field, shape) = label_field_for(id).unwrap_or(("(unknown)", "(unknown)"));
        let joined = rows.iter().any(|row| &row.miner_id == id);
        println!(
            "{id:<10} {field:<24} {shape:<22} {}",
            if joined { "yes" } else { "NO" }
        );
    }

    let container_miners: Vec<&String> = observed_miner_ids
        .iter()
        .filter(|id| {
            label_field_for(id)
                .map(|(_, shape)| shape.contains("object") || shape.contains("array"))
                .unwrap_or(false)
        })
        .collect();
    if !container_miners.is_empty() {
        println!();
        println!("WARNING: label_field names a container, not a scalar, for miner(s):");
        for id in &container_miners {
            let (field, shape) = label_field_for(id).unwrap_or(("?", "?"));
            println!("  miner {id}: label_field {field:?} is a {shape}");
        }
        println!("The scoring module receives ONE extracted value and its parser");
        println!("assumes a scalar. A standardiser that passed the labelled field");
        println!("through unchanged would hand the module a JSON blob, which");
        println!("parses as no quantity and scores 0.0 for a correct answer.");
        println!("This evaluation extracts the temperature itself, so it does not");
        println!("hit that path. A validator using the registry mapping would.");
    }

    println!();
    println!("=== ROWS DROPPED ===");
    if drops.reasons.is_empty() {
        println!("none");
    } else {
        for (reason, count) in &drops.reasons {
            println!("  {reason:<44} {count}");
        }
    }

    println!();
    println!("=== PER-MINER ACCURACY AGAINST ARCHIVE ACTUALS ===");
    println!("error is the miner's own claimed Celsius minus the archive actual");
    println!("at the geocoded city and the ask hour");
    println!();
    let accuracies = accuracy_by_miner(rows);
    println!(
        "{:<10} {:<22} {:>5} {:>9} {:>9} {:>9} {:>8} {:>8}",
        "miner_id", "slug", "rows", "mean|e|", "med|e|", "mean e", "worst", "drift m"
    );
    for accuracy in &accuracies {
        println!(
            "{:<10} {:<22} {:>5} {:>9.3} {:>9.3} {:>9.3} {:>8.2} {:>8.1}",
            accuracy.miner_id,
            accuracy.miner_slug,
            accuracy.rows,
            accuracy.mean_absolute_error,
            accuracy.median_absolute_error,
            accuracy.mean_signed_error,
            accuracy.worst_absolute_error,
            accuracy.mean_drift_minutes
        );
    }
    if accuracies.is_empty() {
        println!("(no rows)");
    }

    println!();
    println!("=== SPEND ===");
    println!(
        "settled: {settled_units} smallest units ({:.2} USDC)",
        (settled_units as f64) / 1_000_000.0
    );

    println!();
    println!("=== PAIR YIELD AGAINST THE PROBE'S PREDICTION ===");
    let miss = (1.0 - minority_share).powi(asks_per_city as i32);
    let expected = (cities as f64) * (1.0 - miss);
    println!(
        "the probe saw a minority miner at {:.1}% of routing",
        100.0 * minority_share
    );
    println!(
        "so {asks_per_city} asks should miss it in {:.2}% of cities",
        100.0 * miss
    );
    println!("expected paired cities: {expected:.2} of {cities}");
    println!("observed paired cities: {paired_cities} of {cities}");

    // One city short of the expectation is noise at this sample size.
    // Two or more short means the split does not hold and the design
    // assumption is wrong.
    let met = (paired_cities as f64) >= expected - 1.0;
    if met {
        println!("VERDICT: the yield matches the prediction.");
    } else {
        println!("VERDICT: the yield is BELOW what the 23/7 split predicts.");
        println!("Routing is not behaving as the probe measured. Stop and");
        println!("re-measure the split before buying more asks.");
    }
    met
}

/// This function computes accuracy per miner.
pub fn accuracy_by_miner(rows: &[JoinedRow]) -> Vec<MinerAccuracy> {
    let mut grouped: BTreeMap<(String, String), Vec<&JoinedRow>> = BTreeMap::new();
    for row in rows {
        grouped
            .entry((row.miner_id.clone(), row.miner_slug.clone()))
            .or_default()
            .push(row);
    }

    let mut out = Vec::new();
    for ((miner_id, miner_slug), group) in grouped {
        let mut absolute: Vec<f64> = group
            .iter()
            .map(|row| (row.miner_c - row.actual_c).abs())
            .collect();
        let signed: f64 = group
            .iter()
            .map(|row| row.miner_c - row.actual_c)
            .sum::<f64>()
            / (group.len() as f64);
        let drift: f64 = group
            .iter()
            .map(|row| row.drift_minutes as f64)
            .sum::<f64>()
            / (group.len() as f64);
        let mean = absolute.iter().sum::<f64>() / (absolute.len() as f64);
        absolute.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = absolute[absolute.len() / 2];
        let worst = *absolute.last().unwrap_or(&0.0);

        out.push(MinerAccuracy {
            miner_id,
            miner_slug,
            rows: group.len(),
            mean_absolute_error: mean,
            median_absolute_error: median,
            mean_signed_error: signed,
            worst_absolute_error: worst,
            mean_drift_minutes: drift,
        });
    }
    out.sort_by(|a, b| {
        a.mean_absolute_error
            .partial_cmp(&b.mean_absolute_error)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(city: &str, miner: &str, miner_c: f64, actual_c: f64) -> JoinedRow {
        JoinedRow {
            city_key: city.to_string(),
            miner_id: miner.to_string(),
            miner_slug: format!("slug{miner}"),
            miner_c,
            actual_c,
            drift_minutes: 10,
            line: String::new(),
        }
    }

    #[test]
    fn accuracy_is_computed_per_miner() {
        let rows = vec![
            row("tokyo", "211", 20.0, 19.0),
            row("tokyo", "211", 21.0, 19.0),
            row("tokyo", "212", 19.5, 19.0),
        ];
        let accuracies = accuracy_by_miner(&rows);
        assert_eq!(accuracies.len(), 2);
        // Sorted best first: 212 has a 0.5 error, 211 averages 1.5.
        assert_eq!(accuracies[0].miner_id, "212");
        assert!((accuracies[0].mean_absolute_error - 0.5).abs() < 1e-9);
        assert!((accuracies[1].mean_absolute_error - 1.5).abs() < 1e-9);
    }

    #[test]
    fn a_signed_error_shows_a_warm_bias() {
        let rows = vec![
            row("tokyo", "211", 21.0, 19.0),
            row("tokyo", "211", 22.0, 19.0),
        ];
        let accuracies = accuracy_by_miner(&rows);
        assert!(
            accuracies[0].mean_signed_error > 2.0,
            "a miner reading high must show a positive signed error"
        );
    }

    #[test]
    fn one_miner_over_a_city_is_not_a_pair() {
        // Twenty rows from one miner rank nothing. This is the whole
        // definition the report turns on.
        let rows: Vec<JoinedRow> = (0..20).map(|_| row("tokyo", "211", 20.0, 19.0)).collect();
        let mut by_city: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for r in &rows {
            by_city
                .entry(r.city_key.clone())
                .or_default()
                .insert(r.miner_id.clone());
        }
        assert_eq!(by_city["tokyo"].len(), 1);
    }
}
