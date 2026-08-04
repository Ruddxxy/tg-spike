//! This test file checks that the calibrated archetypes are truly
//! calibrated on the balanced data set, and that the skewed data set
//! deviates from calibration in the exact way that Bayes' rule
//! predicts.
//!
//! "Calibrated" means: among every response with a reported confidence
//! near `r`, close to `r` of those items truly have label 1.
//!
//! `NoisyGood` and `NoisyMediocre` build their confidence from the item
//! signal only, so on the skewed data set their reported value drifts
//! away from the empirical hit rate. `BayesCalibratedGood` also applies
//! the class base rate as a prior, so on the skewed data set its own
//! reported value must match the empirical hit rate directly. That gap
//! between "signal only" and "signal plus base rate" is the entire
//! reason `BayesCalibratedGood` exists.

use miner_sim::archetype::{bayes_posterior, responses_for};
use miner_sim::dataset;
use miner_sim::types::{Archetype, Dataset, DatasetShape, ResponseKind, ResponseSeed};

/// The item count for every check in this file. A large count keeps
/// the bins full, so the sampling error bound stays tight.
const ITEM_COUNT: usize = 200_000;

/// The seed for the balanced data set.
const BALANCED_DATASET_SEED: u64 = 4_001;
/// The response seed for the balanced check.
///
/// This value must differ from `BALANCED_DATASET_SEED`. See the trap
/// note on `miner_sim::archetype::responses_for`: a matching seed
/// lines up a miner's draws with its own item's draws and breaks the
/// numbers this test checks.
const BALANCED_RESPONSE_SEED: u64 = 909_004_001;

/// The seed for the skewed data set.
const SKEWED_DATASET_SEED: u64 = 4_002;
/// The response seed for the skewed check. This value must differ
/// from `SKEWED_DATASET_SEED`, for the same reason as above.
const SKEWED_RESPONSE_SEED: u64 = 909_004_002;

/// The fewest items a bin needs before this test trusts its numbers.
const MIN_BIN_COUNT: usize = 500;

/// The running total for one bin of reported confidence.
struct BinStat {
    /// The count of answers that fall in this bin.
    count: usize,
    /// The sum of every reported confidence in this bin.
    sum_reported: f64,
    /// The count of items in this bin with a true label of 1.
    label_one: usize,
}

/// This function makes 10 empty bins, one for every tenth of the
/// confidence range 0.0 up to 1.0.
fn empty_bins() -> [BinStat; 10] {
    std::array::from_fn(|_| BinStat {
        count: 0,
        sum_reported: 0.0,
        label_one: 0,
    })
}

/// This function finds the bin index for a confidence value.
///
/// A confidence of exactly 1.0 would map to index 10, out of range,
/// so the function clamps the index to 9. Every value in the range 0.0
/// up to 1.0 gets a valid bin.
fn bin_index(confidence: f64) -> usize {
    let raw = (confidence * 10.0).floor() as i64;
    raw.clamp(0, 9) as usize
}

/// This function bins every well formed answer of one archetype on one
/// data set, by reported confidence.
fn build_bins(
    archetype: Archetype,
    dataset: &Dataset,
    response_seed: ResponseSeed,
) -> [BinStat; 10] {
    let responses = responses_for(archetype, dataset, response_seed);
    let mut bins = empty_bins();
    for (item, response) in dataset.items.iter().zip(responses.iter()) {
        if let ResponseKind::Answer { confidence, .. } = response.kind {
            let bin = &mut bins[bin_index(confidence)];
            bin.count += 1;
            bin.sum_reported += confidence;
            if item.label == 1 {
                bin.label_one += 1;
            }
        }
    }
    bins
}

/// This function gives the sampling error bound for a bin.
///
/// The bound uses three standard errors of a binomial proportion, plus
/// a small fixed slack of 0.01. Three standard errors gives a very low
/// chance of a false failure from sampling noise alone.
fn sampling_bound(empirical: f64, count: usize) -> f64 {
    3.0 * (empirical * (1.0 - empirical) / count as f64).sqrt() + 0.01
}

#[test]
fn balanced_calibration_matches_reported_confidence() {
    let dataset = dataset::generate(DatasetShape::Balanced, ITEM_COUNT, BALANCED_DATASET_SEED);

    for archetype in Archetype::ALL {
        if !archetype.is_calibrated() {
            continue;
        }
        let bins = build_bins(
            archetype,
            &dataset,
            ResponseSeed::new_unchecked(BALANCED_RESPONSE_SEED),
        );

        println!("--- {archetype:?} on the balanced data set ---");
        println!(
            "{:<14}{:>10}{:>16}{:>16}{:>10}",
            "bin", "count", "mean_reported", "empirical", "bound"
        );
        for (i, bin) in bins.iter().enumerate() {
            let low = i as f64 / 10.0;
            let high = (i + 1) as f64 / 10.0;
            if bin.count < MIN_BIN_COUNT {
                println!(
                    "[{low:.1}, {high:.1}) skipped: count {} is under the floor of {MIN_BIN_COUNT}",
                    bin.count
                );
                continue;
            }
            let mean_reported = bin.sum_reported / bin.count as f64;
            let empirical = bin.label_one as f64 / bin.count as f64;
            let bound = sampling_bound(empirical, bin.count);
            println!(
                "[{low:.1}, {high:.1}) {:>10}{:>16.4}{:>16.4}{:>10.4}",
                bin.count, mean_reported, empirical, bound
            );
            let gap = (mean_reported - empirical).abs();
            assert!(
                gap < bound,
                "{archetype:?} bin [{low:.1}, {high:.1}): mean reported {mean_reported:.4} \
                 and empirical fraction {empirical:.4} differ by {gap:.4}, over the bound {bound:.4}"
            );
        }
    }
}

#[test]
fn skewed_calibration_deviates_the_way_bayes_rule_predicts() {
    let dataset = dataset::generate(DatasetShape::Skewed, ITEM_COUNT, SKEWED_DATASET_SEED);
    let base_rate = DatasetShape::Skewed.base_rate();

    for archetype in Archetype::ALL {
        if !archetype.is_calibrated() {
            continue;
        }
        let bins = build_bins(
            archetype,
            &dataset,
            ResponseSeed::new_unchecked(SKEWED_RESPONSE_SEED),
        );

        println!("--- {archetype:?} on the skewed data set ---");
        println!(
            "{:<14}{:>10}{:>16}{:>16}{:>16}{:>10}",
            "bin", "count", "mean_reported", "empirical", "predicted", "bound"
        );
        for (i, bin) in bins.iter().enumerate() {
            let low = i as f64 / 10.0;
            let high = (i + 1) as f64 / 10.0;
            if bin.count < MIN_BIN_COUNT {
                println!(
                    "[{low:.1}, {high:.1}) skipped: count {} is under the floor of {MIN_BIN_COUNT}",
                    bin.count
                );
                continue;
            }
            let mean_reported = bin.sum_reported / bin.count as f64;
            let empirical = bin.label_one as f64 / bin.count as f64;
            // `BayesCalibratedGood` already applies the base rate as a
            // prior inside the archetype, so its own reported value is
            // the prediction: it must match the empirical fraction
            // directly, with no extra transform. `NoisyGood` and
            // `NoisyMediocre` know nothing about the base rate, so this
            // test applies `bayes_posterior` to find what a Bayesian
            // observer of their reported value `r` would predict. A
            // calibrated miner's reported value equals `r` only at base
            // rate 0.50. At this base rate the doc comment on
            // `calibrated_answer` gives the general formula, and
            // `bayes_posterior` is that same formula.
            let predicted = if archetype == Archetype::BayesCalibratedGood {
                mean_reported
            } else {
                bayes_posterior(mean_reported, base_rate)
            };
            let bound = sampling_bound(empirical, bin.count);
            println!(
                "[{low:.1}, {high:.1}) {:>10}{:>16.4}{:>16.4}{:>16.4}{:>10.4}",
                bin.count, mean_reported, empirical, predicted, bound
            );
            let gap_to_predicted = (predicted - empirical).abs();
            assert!(
                gap_to_predicted < bound,
                "{archetype:?} bin [{low:.1}, {high:.1}): predicted fraction {predicted:.4} \
                 and empirical fraction {empirical:.4} differ by {gap_to_predicted:.4}, over the bound {bound:.4}"
            );
        }
    }
}
