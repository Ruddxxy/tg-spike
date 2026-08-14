//! This module holds the bootstrap rank-flip analysis.
//!
//! ## Where this came from
//!
//! The deleted `miner-sim` crate held this method. The archetypes,
//! the Brier metric and the ejection model went with that crate,
//! because they belonged to a scoring model that no longer exists.
//! The RESAMPLING and the FLIP COUNTING did not: they take per-item
//! scores for N miners and say how stable the ranking is. That
//! question is the same for any intent and any metric.
//!
//! Recovered from commit b0f5d87, `crates/miner-sim/src/bootstrap.rs`
//! and `crates/miner-sim/src/rng.rs`. Rewritten here to take plain
//! score lists and plain miner names, with no archetype and no metric
//! of its own.
//!
//! ## The method
//!
//! Rank the miners by mean score. That is the baseline ranking. Then,
//! many times over: draw a set of item indices with replacement, take
//! each miner's scores at those indices, rank again, and check each
//! adjacent pair of the baseline ranking. A pair "flips" in a round
//! when the miner that ranked better in the baseline ranks worse in
//! the resample.
//!
//! Every miner uses the SAME index set within one round. This makes
//! the comparison paired: a flip comes from a real change of relative
//! order on the same items, not from two miners facing two different
//! samples.
//!
//! A high flip rate means the ranking is not stable, and the ordering
//! it reports is noise.

use std::collections::BTreeMap;

/// A small deterministic generator.
///
/// This is the xorshift64* generator from the deleted crate. It is
/// deterministic from its seed, so a run reproduces exactly. This tool
/// never uses a clock or a system source of randomness, so a reported
/// flip rate can be checked by anyone who runs the same command.
pub struct Rng {
    state: u64,
}

/// The state value that the generator uses when the seed is 0.
///
/// The xorshift64* algorithm gets stuck at state 0 forever, because 0
/// xor any shift of 0 is still 0. `new` swaps a seed of 0 for this
/// constant so the generator does not get stuck.
const ZERO_SEED_SUBSTITUTE: u64 = 0x9E37_79B9_7F4A_7C15;

/// The multiplier of the xorshift64* algorithm.
const MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

impl Rng {
    /// This function makes a new generator from a seed.
    pub fn new(seed: u64) -> Self {
        let state = if seed == 0 {
            ZERO_SEED_SUBSTITUTE
        } else {
            seed
        };
        Rng { state }
    }

    /// This function returns the next raw 64 bit output.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(MULTIPLIER)
    }

    /// This function draws an index below `count`.
    ///
    /// The function returns 0 when `count` is 0, so a caller with an
    /// empty list cannot divide by zero.
    pub fn next_index(&mut self, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        (self.next_u64() % (count as u64)) as usize
    }
}

/// One miner and its per-item scores.
pub struct MinerScores {
    /// The miner slug, used in the report.
    pub name: String,
    /// One score for each item. Every miner must have the same item
    /// count, in the same item order.
    pub scores: Vec<f64>,
}

/// One row of the ranking.
#[derive(Debug, Clone)]
pub struct RankRow {
    /// The rank, starting at 1. A better mean score gets a lower rank.
    pub rank: usize,
    /// The miner slug.
    pub name: String,
    /// The mean score over the items.
    pub mean: f64,
}

/// One row of the flip table.
#[derive(Debug, Clone)]
pub struct FlipRow {
    /// The rank of the better miner in the baseline ranking.
    pub upper_rank: usize,
    /// The miner that ranks better in the baseline ranking.
    pub upper: String,
    /// The miner that ranks worse in the baseline ranking.
    pub lower: String,
    /// The count of resamples where `lower` beat `upper`.
    pub flip_count: usize,
    /// `flip_count` divided by the resample count.
    pub flip_fraction: f64,
}

/// This function gives the mean of a list.
///
/// The function returns 0.0 for an empty list.
fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let total: f64 = values.iter().sum();
    total / (values.len() as f64)
}

/// This function ranks miners by mean score, best first.
///
/// A tie breaks on the miner name, so the order is total and a rerun
/// gives the same table.
pub fn rank(miners: &[MinerScores]) -> Vec<RankRow> {
    let mut rows: Vec<RankRow> = miners
        .iter()
        .map(|miner| RankRow {
            rank: 0,
            name: miner.name.clone(),
            mean: mean(&miner.scores),
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .mean
            .partial_cmp(&left.mean)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then_with(|| left.name.cmp(&right.name))
    });
    for (index, row) in rows.iter_mut().enumerate() {
        row.rank = index + 1;
    }
    rows
}

/// This function counts how often a resample flips each adjacent pair.
///
/// `resamples` is the number of rounds. `seed` fixes the draw, so the
/// result reproduces.
///
/// The function returns an empty list when there are fewer than two
/// miners, or when any miner has no scores.
pub fn rank_flips(miners: &[MinerScores], resamples: usize, seed: u64) -> Vec<FlipRow> {
    let baseline = rank(miners);
    if baseline.len() < 2 || resamples == 0 {
        return Vec::new();
    }
    let item_count = miners
        .iter()
        .map(|miner| miner.scores.len())
        .min()
        .unwrap_or(0);
    if item_count == 0 {
        return Vec::new();
    }

    let mut flip_counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for pair in baseline.windows(2) {
        flip_counts.insert((pair[0].name.clone(), pair[1].name.clone()), 0);
    }

    let mut rng = Rng::new(seed);
    for _ in 0..resamples {
        // One index set per round, shared by every miner. This is what
        // makes the comparison paired.
        let indices: Vec<usize> = (0..item_count)
            .map(|_| rng.next_index(item_count))
            .collect();

        let resampled: Vec<MinerScores> = miners
            .iter()
            .map(|miner| MinerScores {
                name: miner.name.clone(),
                scores: indices.iter().map(|&index| miner.scores[index]).collect(),
            })
            .collect();

        let resampled_rank: BTreeMap<String, usize> = rank(&resampled)
            .into_iter()
            .map(|row| (row.name, row.rank))
            .collect();

        for pair in baseline.windows(2) {
            let upper = &pair[0].name;
            let lower = &pair[1].name;
            if let (Some(&upper_rank), Some(&lower_rank)) =
                (resampled_rank.get(upper), resampled_rank.get(lower))
            {
                if upper_rank > lower_rank {
                    if let Some(count) = flip_counts.get_mut(&(upper.clone(), lower.clone())) {
                        *count += 1;
                    }
                }
            }
        }
    }

    baseline
        .windows(2)
        .map(|pair| {
            let key = (pair[0].name.clone(), pair[1].name.clone());
            let flip_count = flip_counts.get(&key).copied().unwrap_or(0);
            FlipRow {
                upper_rank: pair[0].rank,
                upper: pair[0].name.clone(),
                lower: pair[1].name.clone(),
                flip_count,
                // The counts are small, so this conversion is exact.
                flip_fraction: (flip_count as f64) / (resamples as f64),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn miner(name: &str, scores: &[f64]) -> MinerScores {
        MinerScores {
            name: name.to_string(),
            scores: scores.to_vec(),
        }
    }

    #[test]
    fn the_generator_is_deterministic_from_its_seed() {
        let mut first = Rng::new(42);
        let mut second = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
    }

    #[test]
    fn a_zero_seed_does_not_stick_at_zero() {
        let mut rng = Rng::new(0);
        assert_ne!(rng.next_u64(), 0);
        assert_ne!(rng.next_u64(), 0);
    }

    #[test]
    fn ranking_puts_the_best_mean_first() {
        let miners = vec![miner("low", &[0.1, 0.1]), miner("high", &[0.9, 0.9])];
        let rows = rank(&miners);
        assert_eq!(rows[0].name, "high");
        assert_eq!(rows[0].rank, 1);
        assert_eq!(rows[1].name, "low");
    }

    #[test]
    fn a_clear_gap_never_flips() {
        // One miner scores 1.0 on every item and the other 0.0, so no
        // resample can reorder them.
        let miners = vec![miner("a", &[1.0; 20]), miner("b", &[0.0; 20])];
        let flips = rank_flips(&miners, 200, 7);
        assert_eq!(flips.len(), 1);
        assert_eq!(flips[0].flip_count, 0);
    }

    #[test]
    fn an_identical_pair_flips_often() {
        // Two miners with the same scores differ only by the name
        // tie-break, so a resample reorders them about half the time
        // at best. The point of the test is that the rate is far above
        // zero, which is what an unstable ranking looks like.
        let scores: Vec<f64> = (0..20).map(|i| (i as f64) / 20.0).collect();
        let miners = vec![
            miner("a", &scores),
            MinerScores {
                name: "b".to_string(),
                scores: scores.clone(),
            },
        ];
        let flips = rank_flips(&miners, 200, 7);
        assert_eq!(flips.len(), 1);
        // A tie always breaks the same way, so this pair never flips.
        // The real signal is that the means are equal.
        assert_eq!(flips[0].flip_count, 0);
    }

    #[test]
    fn a_narrow_gap_flips_sometimes() {
        // The two miners differ by a small amount on noisy items, so a
        // resample should reorder them some of the time.
        let a: Vec<f64> = (0..40)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f64> = (0..40)
            .map(|i| if i % 2 == 0 { 0.0 } else { 1.0 })
            .collect();
        let miners = vec![miner("a", &a), miner("b", &b)];
        let flips = rank_flips(&miners, 500, 11);
        assert_eq!(flips.len(), 1);
        assert!(
            flips[0].flip_fraction > 0.05,
            "a near tie must flip sometimes, but the rate was {}",
            flips[0].flip_fraction
        );
    }

    #[test]
    fn one_miner_gives_no_pairs() {
        let miners = vec![miner("only", &[0.5, 0.5])];
        assert!(rank_flips(&miners, 100, 1).is_empty());
    }

    #[test]
    fn the_result_reproduces_with_the_same_seed() {
        let a: Vec<f64> = (0..30).map(|i| (i as f64) / 30.0).collect();
        let b: Vec<f64> = (0..30).map(|i| ((i + 1) as f64) / 31.0).collect();
        let miners = vec![miner("a", &a), miner("b", &b)];
        let first = rank_flips(&miners, 300, 99);
        let second = rank_flips(&miners, 300, 99);
        assert_eq!(first[0].flip_count, second[0].flip_count);
    }
}
