//! This module reduces the scored corpus into the report tables.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;

/// One scored row, as the wazero runner writes it.
#[derive(Debug, Deserialize, Clone)]
pub struct ScoredRow {
    /// The corpus row number. The known-bad report names rows by it.
    #[allow(dead_code)]
    pub row_id: usize,
    /// The miner slug.
    pub miner_slug: String,
    /// The paraphrase cluster, or an empty text.
    pub cluster_id: String,
    /// The archive actual, in Celsius.
    pub actual_c: f64,
    /// The miner's own claimed value, in Celsius.
    pub miner_c: f64,
    /// Our score against the bare rendering.
    pub ours_bare: f64,
    /// Our score against the prose rendering.
    pub ours_prose: f64,
    /// Our score against the JSON rendering.
    pub ours_json: f64,
    /// The reference score against the bare rendering.
    pub ref_bare: f64,
    /// The reference score against the prose rendering.
    pub ref_prose: f64,
    /// The reference score against the JSON rendering.
    pub ref_json: f64,
}

/// This function reads the scored rows.
pub fn load_scores(path: &Path) -> std::io::Result<Vec<ScoredRow>> {
    let reader = BufReader::new(File::open(path)?);
    let mut rows = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(row) => rows.push(row),
            Err(error) => {
                return Err(std::io::Error::other(format!(
                    "cannot read a scored row: {error}"
                )))
            }
        }
    }
    Ok(rows)
}

/// This function gives the mean of a list.
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.iter().sum::<f64>() / (values.len() as f64)
}

/// This function gives a percentile of a list, by the nearest-rank
/// rule.
///
/// The function sorts a copy, so the caller's list keeps its order.
pub fn percentile(values: &[f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| {
        left.partial_cmp(right)
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    let rank = (fraction * (sorted.len() as f64)).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

/// This function gives the median of a list.
pub fn median(values: &[f64]) -> f64 {
    percentile(values, 0.5)
}

/// This function gives the Pearson correlation of two lists.
///
/// The function returns NaN when either list has no spread, because a
/// correlation is not defined then.
pub fn correlation(left: &[f64], right: &[f64]) -> f64 {
    if left.len() != right.len() || left.len() < 2 {
        return f64::NAN;
    }
    let left_mean = mean(left);
    let right_mean = mean(right);
    let mut covariance = 0.0;
    let mut left_spread = 0.0;
    let mut right_spread = 0.0;
    for (a, b) in left.iter().zip(right.iter()) {
        let da = a - left_mean;
        let db = b - right_mean;
        covariance += da * db;
        left_spread += da * da;
        right_spread += db * db;
    }
    if left_spread <= 0.0 || right_spread <= 0.0 {
        return f64::NAN;
    }
    covariance / (left_spread.sqrt() * right_spread.sqrt())
}

/// This function prints how much a score changes with the rendering.
///
/// The real ground-truth format is undisclosed, so a score that moves
/// with the rendering is a defect. This table measures how far it
/// moves on real data.
pub fn print_rendering_variance(rows: &[ScoredRow]) {
    println!("=== SCORE VARIANCE ACROSS THE THREE GROUND TRUTH RENDERINGS ===");
    println!("n = {} rows", rows.len());
    println!();

    let ours_bare: Vec<f64> = rows.iter().map(|row| row.ours_bare).collect();
    let ours_prose: Vec<f64> = rows.iter().map(|row| row.ours_prose).collect();
    let ours_json: Vec<f64> = rows.iter().map(|row| row.ours_json).collect();
    let ref_bare: Vec<f64> = rows.iter().map(|row| row.ref_bare).collect();
    let ref_prose: Vec<f64> = rows.iter().map(|row| row.ref_prose).collect();
    let ref_json: Vec<f64> = rows.iter().map(|row| row.ref_json).collect();

    println!(
        "{:<12} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "scorer", "bare mean", "prose mean", "json mean", "bare med", "prose med", "json med"
    );
    println!(
        "{:<12} {:>10.6} {:>10.6} {:>10.6} {:>10.6} {:>10.6} {:>10.6}",
        "ours",
        mean(&ours_bare),
        mean(&ours_prose),
        mean(&ours_json),
        median(&ours_bare),
        median(&ours_prose),
        median(&ours_json)
    );
    println!(
        "{:<12} {:>10.6} {:>10.6} {:>10.6} {:>10.6} {:>10.6} {:>10.6}",
        "reference",
        mean(&ref_bare),
        mean(&ref_prose),
        mean(&ref_json),
        median(&ref_bare),
        median(&ref_prose),
        median(&ref_json)
    );
    println!();

    // The per-row spread is the number that matters. A mean that
    // matches can still hide a score that swings row by row.
    let mut ours_spread = Vec::with_capacity(rows.len());
    let mut ref_spread = Vec::with_capacity(rows.len());
    let mut ours_identical = 0usize;
    let mut ref_identical = 0usize;
    for row in rows {
        let ours_max = row.ours_bare.max(row.ours_prose).max(row.ours_json);
        let ours_min = row.ours_bare.min(row.ours_prose).min(row.ours_json);
        let spread = ours_max - ours_min;
        if spread == 0.0 {
            ours_identical += 1;
        }
        ours_spread.push(spread);

        let ref_max = row.ref_bare.max(row.ref_prose).max(row.ref_json);
        let ref_min = row.ref_bare.min(row.ref_prose).min(row.ref_json);
        let ref_gap = ref_max - ref_min;
        if ref_gap == 0.0 {
            ref_identical += 1;
        }
        ref_spread.push(ref_gap);
    }

    println!("per-row spread, max score minus min score over the three renderings:");
    println!(
        "{:<12} {:>10} {:>10} {:>10} {:>14}",
        "scorer", "mean", "median", "p90", "rows identical"
    );
    println!(
        "{:<12} {:>10.6} {:>10.6} {:>10.6} {:>8} ({:>4.1}%)",
        "ours",
        mean(&ours_spread),
        median(&ours_spread),
        percentile(&ours_spread, 0.90),
        ours_identical,
        100.0 * (ours_identical as f64) / (rows.len() as f64)
    );
    println!(
        "{:<12} {:>10.6} {:>10.6} {:>10.6} {:>8} ({:>4.1}%)",
        "reference",
        mean(&ref_spread),
        median(&ref_spread),
        percentile(&ref_spread, 0.90),
        ref_identical,
        100.0 * (ref_identical as f64) / (rows.len() as f64)
    );
}

/// One per-miner accuracy row.
pub struct MinerStats {
    /// The miner slug.
    pub slug: String,
    /// The row count for this miner.
    pub count: usize,
    /// The mean absolute error in Celsius.
    pub error_mean: f64,
    /// The median absolute error in Celsius.
    pub error_median: f64,
    /// The 90th percentile absolute error in Celsius.
    pub error_p90: f64,
    /// The mean of our score.
    pub ours_mean: f64,
    /// The median of our score.
    pub ours_median: f64,
    /// The mean of the reference score.
    pub ref_mean: f64,
    /// The median of the reference score.
    pub ref_median: f64,
    /// The correlation between our score and the negative absolute
    /// error.
    pub ours_correlation: f64,
    /// The correlation between the reference score and the negative
    /// absolute error.
    pub ref_correlation: f64,
}

/// This function computes the per-miner accuracy table.
///
/// The score used here is the BARE rendering. The variance table shows
/// how far the other two renderings sit from it.
pub fn miner_stats(rows: &[ScoredRow]) -> Vec<MinerStats> {
    let mut by_miner: BTreeMap<String, Vec<&ScoredRow>> = BTreeMap::new();
    for row in rows {
        by_miner
            .entry(row.miner_slug.clone())
            .or_default()
            .push(row);
    }

    by_miner
        .into_iter()
        .map(|(slug, group)| {
            let errors: Vec<f64> = group
                .iter()
                .map(|row| (row.miner_c - row.actual_c).abs())
                .collect();
            let negative_errors: Vec<f64> = errors.iter().map(|error| -error).collect();
            let ours: Vec<f64> = group.iter().map(|row| row.ours_bare).collect();
            let reference: Vec<f64> = group.iter().map(|row| row.ref_bare).collect();

            MinerStats {
                slug,
                count: group.len(),
                error_mean: mean(&errors),
                error_median: median(&errors),
                error_p90: percentile(&errors, 0.90),
                ours_mean: mean(&ours),
                ours_median: median(&ours),
                ref_mean: mean(&reference),
                ref_median: median(&reference),
                ours_correlation: correlation(&ours, &negative_errors),
                ref_correlation: correlation(&reference, &negative_errors),
            }
        })
        .collect()
}

/// This function prints the per-miner accuracy table.
pub fn print_miner_stats(rows: &[ScoredRow]) {
    println!("=== PER MINER ACCURACY, AND SCORE AGAINST REAL ERROR ===");
    println!("score column uses the bare ground-truth rendering.");
    println!("error is |miner value - archive actual|, in Celsius.");
    println!();
    println!(
        "{:<22} {:>6} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "miner",
        "n",
        "err mean",
        "err med",
        "err p90",
        "ours mean",
        "ours med",
        "ref mean",
        "ref med"
    );
    let stats = miner_stats(rows);
    for row in &stats {
        println!(
            "{:<22} {:>6} {:>9.3} {:>9.3} {:>9.3} {:>9.6} {:>9.6} {:>9.6} {:>9.6}",
            row.slug,
            row.count,
            row.error_mean,
            row.error_median,
            row.error_p90,
            row.ours_mean,
            row.ours_median,
            row.ref_mean,
            row.ref_median
        );
    }

    println!();
    println!("correlation between score and NEGATIVE absolute error:");
    println!("a scorer that tracks real accuracy gives a positive value near 1.");
    println!(
        "{:<22} {:>6} {:>14} {:>14}",
        "miner", "n", "ours", "reference"
    );
    for row in &stats {
        println!(
            "{:<22} {:>6} {:>14.4} {:>14.4}",
            row.slug, row.count, row.ours_correlation, row.ref_correlation
        );
    }

    // The pooled correlation over every row.
    let errors: Vec<f64> = rows
        .iter()
        .map(|row| -((row.miner_c - row.actual_c).abs()))
        .collect();
    let ours: Vec<f64> = rows.iter().map(|row| row.ours_bare).collect();
    let reference: Vec<f64> = rows.iter().map(|row| row.ref_bare).collect();
    println!(
        "{:<22} {:>6} {:>14.4} {:>14.4}",
        "ALL POOLED",
        rows.len(),
        correlation(&ours, &errors),
        correlation(&reference, &errors)
    );
}
