//! This module runs a paired bootstrap over the already scored items.
//!
//! Every function in this module reuses the per-item `scores` that
//! `scoring::score_miner` already got from the WASM module. No function
//! in this module calls the module again. A resample only draws item
//! indices and reads scores that a real score run already gave.
//!
//! # Score direction
//!
//! A HIGH score is good. Read the note on score direction in
//! `types.rs`. Every gap and every confidence range in this module is
//! `mean(better miner) - mean(worse miner)`, so a positive value always
//! means "the better miner truly scores higher".

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use host_runner::instance::ScriptInstance;

use crate::leaderboard;
use crate::rng::Rng;
use crate::types::{Archetype, DatasetShape, LeaderboardRow, Metric, MinerResult, ResponseSeed};

/// One row of the rank flip table.
///
/// The row covers one adjacent pair in the baseline leaderboard. It
/// says how often a bootstrap resample put the pair the other way
/// round.
#[derive(Debug, Clone)]
pub struct FlipRow {
    /// The rank of the better miner in the baseline leaderboard.
    pub upper_rank: usize,
    /// The archetype that ranks better in the baseline leaderboard.
    pub upper: Archetype,
    /// The archetype that ranks worse in the baseline leaderboard.
    pub lower: Archetype,
    /// The count of resamples where `lower` beat `upper`.
    pub flip_count: usize,
    /// `flip_count` divided by the resample count.
    pub flip_fraction: f64,
}

/// This function finds how often a bootstrap resample flips each
/// adjacent pair of the baseline leaderboard.
///
/// The function draws one set of item indices per resample round, with
/// replacement, using [`Rng::next_index`]. Every miner uses the SAME
/// index set in one round. This makes the comparison paired: a flip
/// comes from a real change in relative order on the resampled items,
/// not from each miner facing a different sample.
///
/// For each round the function rebuilds a full leaderboard from the
/// resampled scores, with [`leaderboard::build`]. It reads this
/// leaderboard is the source of the resampled rank of each archetype.
/// It uses the same rank and tie-break rules as the real leaderboard,
/// so a flip here means the same thing a flip would mean in a real
/// report.
///
/// For each pair of neighbours in `baseline` (rank 1 with rank 2, rank
/// 2 with rank 3, and so on) the function counts a round as a flip when
/// the resampled rank of the pair's better archetype is worse than the
/// resampled rank of the pair's worse archetype.
///
/// This function does not apply the ejection rule. It always runs the
/// `ScoreAndKeep` comparison, over whatever `results` the caller gives
/// it. The `first_failure` field of each resampled result carries over
/// unchanged from the input; it is inert here, because [`leaderboard::build`]
/// does not read it.
///
/// The function returns an empty list when `baseline` has fewer than 2
/// rows, or when a result has no scores.
#[must_use]
pub fn rank_flips(
    results: &[MinerResult],
    baseline: &[LeaderboardRow],
    resamples: usize,
    seed: u64,
) -> Vec<FlipRow> {
    let n = results.iter().map(|r| r.scores.len()).min().unwrap_or(0);
    if n == 0 || baseline.len() < 2 {
        return Vec::new();
    }

    let mut flip_counts: BTreeMap<(Archetype, Archetype), usize> = BTreeMap::new();
    for pair in baseline.windows(2) {
        flip_counts.insert((pair[0].archetype, pair[1].archetype), 0);
    }

    let mut rng = Rng::new(seed);
    for _ in 0..resamples {
        let indices: Vec<usize> = (0..n).map(|_| rng.next_index(n)).collect();

        let resampled_results: Vec<MinerResult> = results
            .iter()
            .map(|result| MinerResult {
                archetype: result.archetype,
                scores: indices.iter().map(|&i| result.scores[i]).collect(),
                n_malformed: result.n_malformed,
                n_abstained: result.n_abstained,
                first_failure: result.first_failure,
            })
            .collect();
        let resampled_rows = leaderboard::build(&resampled_results);

        let mut resampled_rank: BTreeMap<Archetype, usize> = BTreeMap::new();
        for row in &resampled_rows {
            resampled_rank.insert(row.archetype, row.rank);
        }

        for pair in baseline.windows(2) {
            let upper = pair[0].archetype;
            let lower = pair[1].archetype;
            if let (Some(&upper_rank), Some(&lower_rank)) =
                (resampled_rank.get(&upper), resampled_rank.get(&lower))
            {
                if upper_rank > lower_rank {
                    if let Some(count) = flip_counts.get_mut(&(upper, lower)) {
                        *count += 1;
                    }
                }
            }
        }
    }

    baseline
        .windows(2)
        .map(|pair| {
            let upper = pair[0].archetype;
            let lower = pair[1].archetype;
            let flip_count = flip_counts.get(&(upper, lower)).copied().unwrap_or(0);
            FlipRow {
                upper_rank: pair[0].rank,
                upper,
                lower,
                flip_count,
                flip_fraction: flip_count as f64 / resamples as f64,
            }
        })
        .collect()
}

/// This function makes a text table of the rank flip rows.
#[must_use]
pub fn render_flips(rows: &[FlipRow], title: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "{title}");
    let _ = writeln!(
        out,
        "{:<6} {:<20} {:<20} {:>12} {:>14}",
        "rank", "upper", "lower", "flip_count", "flip_fraction"
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{:<6} {:<20} {:<20} {:>12} {:>14.6}",
            row.upper_rank,
            row.upper.name(),
            row.lower.name(),
            row.flip_count,
            row.flip_fraction
        );
    }
    out
}

/// A report of the gap between the top two miners at one sample size.
#[derive(Debug, Clone)]
pub struct GapReport {
    /// The item count of the data set that made this report.
    pub sample_size: usize,
    /// The archetype with the best (highest) mean score.
    pub top: Archetype,
    /// The archetype with the second best mean score.
    pub second: Archetype,
    /// `top` mean score minus `second` mean score. A positive value
    /// means `top` truly scores better, because a HIGH score is good.
    pub gap: f64,
    /// The 2.5th percentile of the bootstrap gap distribution.
    pub ci_low: f64,
    /// The 97.5th percentile of the bootstrap gap distribution.
    pub ci_high: f64,
    /// True when `ci_low` is greater than 0.0. A true value means the
    /// gap is real: even the low end of the confidence range still
    /// favours `top`.
    pub separated: bool,
}

/// This function finds the nearest-rank percentile of a sorted list.
///
/// `sorted` must already be sorted in ascending order. `p` is a
/// percentage from 0.0 to 100.0. The function uses the nearest-rank
/// method: it picks the value at position `ceil(p / 100 * n)`, counting
/// from 1, and clamps that position to the valid range. This method is
/// simple and needs no interpolation between two array slots.
///
/// The function returns 0.0 for an empty list.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let n = sorted.len();
    let raw_rank = (p / 100.0 * n as f64).ceil() as i64;
    let clamped_rank = raw_rank.clamp(1, n as i64);
    sorted[(clamped_rank - 1) as usize]
}

/// This function scores every archetype on a fresh data set of one
/// size, then bootstraps the gap between the top two miners.
///
/// The function makes a new data set of `size` items with
/// [`crate::dataset::generate`], makes responses for every archetype in
/// `archetypes`, and scores every response through `instance` with
/// `metric`. It finds the top two archetypes by mean score, using the
/// DESCENDING order of [`leaderboard::build`], so `top` is the archetype
/// with the highest mean score. It then resamples item indices
/// `resamples` times, with replacement, and finds the 95 percent
/// confidence range of the gap `mean(top) - mean(second)` over those
/// resamples.
///
/// The function derives a data set seed from `seed` and `size`, so
/// every size in a ladder gets its own data set, and the same `seed`
/// gives the same ladder every time.
///
/// # Errors
///
/// The function returns an error when `archetypes` has fewer than two
/// entries, when a score call into the module fails, or when the top
/// or second archetype is missing from the scored results.
pub fn top_two_gap_at_sizes(
    instance: &mut ScriptInstance,
    shape: DatasetShape,
    archetypes: &[Archetype],
    metric: Metric,
    sizes: &[usize],
    resamples: usize,
    seed: u64,
) -> Result<Vec<GapReport>> {
    if archetypes.len() < 2 {
        bail!("need at least two archetypes to find a top two gap");
    }

    let mut reports = Vec::with_capacity(sizes.len());
    for &size in sizes {
        // A distinct seed per size, mixed with a fixed odd constant, so
        // the data set at one size does not repeat the data set at
        // another size, and the whole ladder stays fixed for a given
        // base seed.
        let dataset_seed = seed ^ (size as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let dataset = crate::dataset::generate(shape, size, dataset_seed);
        // The response seed must differ from the data set seed.
        // `ResponseSeed::derive` applies the fixed mask that keeps the
        // two streams apart. Read the trap note on
        // `archetype::responses_for`. A matching seed lines up the
        // miner correctness draw with the item signal draw, and the
        // miner stops being calibrated.
        let response_seed = ResponseSeed::derive(dataset_seed);

        let mut results = Vec::with_capacity(archetypes.len());
        for &archetype in archetypes {
            let responses = crate::archetype::responses_for(archetype, &dataset, response_seed);
            let result =
                crate::scoring::score_miner(instance, &dataset, archetype, &responses, metric)
                    .with_context(|| {
                        format!(
                            "cannot score archetype {} at sample size {}",
                            archetype.name(),
                            size
                        )
                    })?;
            results.push(result);
        }

        let rows = leaderboard::build(&results);
        if rows.len() < 2 {
            bail!("fewer than two ranked archetypes at sample size {size}");
        }
        let top_row = &rows[0];
        let second_row = &rows[1];
        let top = top_row.archetype;
        let second = second_row.archetype;

        let top_scores = results
            .iter()
            .find(|r| r.archetype == top)
            .map(|r| r.scores.as_slice())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "archetype {} is missing from the scored results at size {}",
                    top.name(),
                    size
                )
            })?;
        let second_scores = results
            .iter()
            .find(|r| r.archetype == second)
            .map(|r| r.scores.as_slice())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "archetype {} is missing from the scored results at size {}",
                    second.name(),
                    size
                )
            })?;

        let n = dataset.items.len();
        let mut rng = Rng::new(dataset_seed ^ 0xABCD_EF01_2345_6789);
        let mut diffs: Vec<f64> = Vec::with_capacity(resamples);
        for _ in 0..resamples {
            let indices: Vec<usize> = (0..n).map(|_| rng.next_index(n)).collect();
            let top_sample: Vec<f64> = indices.iter().map(|&i| top_scores[i]).collect();
            let second_sample: Vec<f64> = indices.iter().map(|&i| second_scores[i]).collect();
            let top_mean = leaderboard::mean(&top_sample);
            let second_mean = leaderboard::mean(&second_sample);
            // A HIGH score is good, so a positive gap must mean `top`
            // truly scores higher than `second`.
            diffs.push(top_mean - second_mean);
        }
        diffs.sort_by(f64::total_cmp);
        let ci_low = percentile(&diffs, 2.5);
        let ci_high = percentile(&diffs, 97.5);
        let gap = top_row.mean_score - second_row.mean_score;
        let separated = ci_low > 0.0;

        reports.push(GapReport {
            sample_size: size,
            top,
            second,
            gap,
            ci_low,
            ci_high,
            separated,
        });
    }

    Ok(reports)
}

/// This function makes a text table of the gap reports.
#[must_use]
pub fn render_gap_reports(rows: &[GapReport], title: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "{title}");
    let _ = writeln!(
        out,
        "{:<12} {:<20} {:<20} {:>10} {:>10} {:>10} {:>10}",
        "sample_size", "top", "second", "gap", "ci_low", "ci_high", "separated"
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{:<12} {:<20} {:<20} {:>10.6} {:>10.6} {:>10.6} {:>10}",
            row.sample_size,
            row.top.name(),
            row.second.name(),
            row.gap,
            row.ci_low,
            row.ci_high,
            row.separated
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_picks_nearest_rank() {
        let sorted = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&sorted, 0.0), 1.0);
        assert_eq!(percentile(&sorted, 100.0), 5.0);
        // ceil(0.5 * 5) = 3, so the third value, 1-based.
        assert_eq!(percentile(&sorted, 50.0), 3.0);
    }

    #[test]
    fn percentile_of_empty_is_zero() {
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    fn miner(archetype: Archetype, scores: Vec<f64>) -> MinerResult {
        MinerResult {
            archetype,
            scores,
            n_malformed: 0,
            n_abstained: 0,
            first_failure: None,
        }
    }

    /// Two miners with a wide, real quality gap. A bootstrap resample
    /// should almost never put the worse miner ahead of the better
    /// one. `a` has the high scores, so it must rank first.
    #[test]
    fn clearly_separated_miners_rarely_flip() {
        let n = 200;
        let mut rng = Rng::new(11);
        let a_scores: Vec<f64> = (0..n).map(|_| rng.uniform_range(0.9, 1.0)).collect();
        let b_scores: Vec<f64> = (0..n).map(|_| rng.uniform_range(0.4, 0.5)).collect();

        let results = vec![
            miner(Archetype::Oracle, a_scores),
            miner(Archetype::Random, b_scores),
        ];
        let baseline = leaderboard::build(&results);
        let flips = rank_flips(&results, &baseline, 500, 999);

        assert_eq!(flips.len(), 1);
        assert!(
            flips[0].flip_fraction < 0.05,
            "flip fraction was {}",
            flips[0].flip_fraction
        );
    }

    /// Two miners built so their full-sample mean score ties exactly,
    /// by a symmetric per-item perturbation that cancels over the full
    /// data set. A bootstrap resample draws an unbalanced mix of the
    /// perturbed items about half the time, so the resampled order
    /// should flip near half of the rounds.
    #[test]
    fn tied_miners_flip_near_half() {
        let n = 200;
        let eps = 0.05;
        let mut rng = Rng::new(300);
        let base: Vec<f64> = (0..n).map(|_| rng.uniform_range(0.3, 0.7)).collect();
        let a_scores: Vec<f64> = base
            .iter()
            .enumerate()
            .map(|(i, &v)| if i % 2 == 0 { v + eps } else { v - eps })
            .collect();
        let b_scores: Vec<f64> = base
            .iter()
            .enumerate()
            .map(|(i, &v)| if i % 2 == 0 { v - eps } else { v + eps })
            .collect();

        let results = vec![
            miner(Archetype::Oracle, a_scores),
            miner(Archetype::Random, b_scores),
        ];
        let baseline = leaderboard::build(&results);
        // The two means cancel out in exact arithmetic, so they must be
        // almost equal. Floating point summation on two different
        // sequences of numbers does not give bit-exact equality, even
        // when the exact sums match, so the check allows a tiny gap
        // instead of a strict `assert_eq!`.
        let gap = (baseline[0].mean_score - baseline[1].mean_score).abs();
        assert!(gap < 1e-9, "the two means should almost tie, gap was {gap}");

        let flips = rank_flips(&results, &baseline, 2000, 1234);
        assert_eq!(flips.len(), 1);
        assert!(
            flips[0].flip_fraction > 0.2 && flips[0].flip_fraction < 0.8,
            "flip fraction was {}",
            flips[0].flip_fraction
        );
    }

    #[test]
    fn rank_flips_of_short_baseline_is_empty() {
        let results = vec![miner(Archetype::Oracle, vec![0.1, 0.2])];
        let baseline = leaderboard::build(&results);
        let flips = rank_flips(&results, &baseline, 100, 1);
        assert!(flips.is_empty());
    }
}
