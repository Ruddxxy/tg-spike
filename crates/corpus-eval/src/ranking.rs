//! This module ranks the miners and measures how stable the ranking
//! is.
//!
//! ## The pairing rule, and why the sample is so small
//!
//! The bootstrap needs PAIRED items: in one resample round every miner
//! must be scored on the same item, or a flip could come from two
//! miners facing two different samples instead of from a real change
//! of order.
//!
//! The item here is a paraphrase CLUSTER, and a miner's score for a
//! cluster is the mean of its rows in that cluster. A cluster only
//! counts when EVERY miner in the comparison answered it. That rule is
//! what keeps the comparison paired, and it is also what makes the
//! sample tiny: the corpus has 40 clusters, only 7 hold more than one
//! miner, and only 3 hold all three miners.
//!
//! The reason is in the feed itself. The three weather miners rarely
//! answer the same question at the same valid time, so there is very
//! little overlap to compare on.
//!
//! Every table this module prints carries its n. A flip rate over 3
//! items is not evidence about which miner is better. It is only
//! evidence that the ranking is or is not stable on those 3 items.

use std::collections::{BTreeMap, BTreeSet};

use crate::bootstrap::{rank, rank_flips, MinerScores};
use crate::stats::ScoredRow;

/// Which score column a ranking uses.
#[derive(Clone, Copy)]
pub enum Scorer {
    /// This submission's module.
    Ours,
    /// The protocol's reference module.
    Reference,
}

impl Scorer {
    /// This function gives the report name of the scorer.
    pub fn label(self) -> &'static str {
        match self {
            Scorer::Ours => "ours",
            Scorer::Reference => "reference",
        }
    }

    /// This function reads the score column for this scorer.
    ///
    /// The column is the bare ground-truth rendering. The variance
    /// table shows how far the other two renderings sit from it.
    fn read(self, row: &ScoredRow) -> f64 {
        match self {
            Scorer::Ours => row.ours_bare,
            Scorer::Reference => row.ref_bare,
        }
    }
}

/// This function builds the per-cluster mean score for each miner.
///
/// The result maps a miner slug to a map of cluster to mean score.
fn cluster_means(rows: &[ScoredRow], scorer: Scorer) -> BTreeMap<String, BTreeMap<String, f64>> {
    let mut totals: BTreeMap<String, BTreeMap<String, (f64, usize)>> = BTreeMap::new();
    for row in rows {
        if row.cluster_id.is_empty() {
            continue;
        }
        let entry = totals
            .entry(row.miner_slug.clone())
            .or_default()
            .entry(row.cluster_id.clone())
            .or_insert((0.0, 0));
        entry.0 += scorer.read(row);
        entry.1 += 1;
    }

    totals
        .into_iter()
        .map(|(miner, clusters)| {
            let means = clusters
                .into_iter()
                .map(|(cluster, (total, count))| {
                    // The counts are small, so this conversion is exact.
                    (cluster, total / (count as f64))
                })
                .collect();
            (miner, means)
        })
        .collect()
}

/// This function finds the clusters that hold more than one miner.
pub fn multi_miner_clusters(rows: &[ScoredRow]) -> BTreeMap<String, BTreeSet<String>> {
    let mut by_cluster: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        if row.cluster_id.is_empty() {
            continue;
        }
        by_cluster
            .entry(row.cluster_id.clone())
            .or_default()
            .insert(row.miner_slug.clone());
    }
    by_cluster.retain(|_, miners| miners.len() > 1);
    by_cluster
}

/// This function builds a paired score set for a group of miners.
///
/// The function keeps only the clusters that EVERY named miner
/// answered. It returns the paired scores and the cluster names it
/// kept, so the caller can report the exact n.
fn paired_scores(
    rows: &[ScoredRow],
    scorer: Scorer,
    miners: &[String],
) -> (Vec<MinerScores>, Vec<String>) {
    let means = cluster_means(rows, scorer);

    let mut shared: Option<BTreeSet<String>> = None;
    for miner in miners {
        let Some(clusters) = means.get(miner) else {
            return (Vec::new(), Vec::new());
        };
        let names: BTreeSet<String> = clusters.keys().cloned().collect();
        shared = Some(match shared {
            None => names,
            Some(existing) => existing.intersection(&names).cloned().collect(),
        });
    }
    let shared: Vec<String> = shared.unwrap_or_default().into_iter().collect();
    if shared.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let scores = miners
        .iter()
        .map(|miner| MinerScores {
            name: miner.clone(),
            scores: shared
                .iter()
                .map(|cluster| {
                    means
                        .get(miner)
                        .and_then(|clusters| clusters.get(cluster))
                        .copied()
                        .unwrap_or(0.0)
                })
                .collect(),
        })
        .collect();

    (scores, shared)
}

/// This function prints the ranking and the flip table for one scorer.
fn print_one(rows: &[ScoredRow], scorer: Scorer, miners: &[String], resamples: usize, seed: u64) {
    let (scores, clusters) = paired_scores(rows, scorer, miners);
    if scores.is_empty() {
        println!(
            "  {}: no cluster is answered by every miner in this group.",
            scorer.label()
        );
        return;
    }

    println!(
        "  {} (n = {} paired clusters -- SMALL):",
        scorer.label(),
        clusters.len()
    );
    for row in rank(&scores) {
        println!(
            "    rank {} {:<24} mean {:.6}",
            row.rank, row.name, row.mean
        );
    }

    let flips = rank_flips(&scores, resamples, seed);
    if flips.is_empty() {
        println!("    no adjacent pair to test.");
        return;
    }
    for flip in flips {
        println!(
            "    flip rate rank {} vs {}: {:.1}% ({} of {} resamples) [{} beats {}]",
            flip.upper_rank,
            flip.upper_rank + 1,
            100.0 * flip.flip_fraction,
            flip.flip_count,
            resamples,
            flip.lower,
            flip.upper
        );
    }
}

/// This function prints the whole ranking-stability section.
pub fn print_rank_flips(rows: &[ScoredRow], resamples: usize, seed: u64) {
    println!("=== RANKING STABILITY ON THE MULTI-MINER CLUSTERS ===");
    println!();

    let clusters = multi_miner_clusters(rows);
    println!(
        "clusters with 2 or more distinct miners: {} -- THIS IS A SMALL SAMPLE",
        clusters.len()
    );
    for (cluster, miners) in &clusters {
        let names: Vec<&str> = miners.iter().map(String::as_str).collect();
        println!("  {cluster:<26} {names:?}");
    }
    println!();
    println!("resamples: {resamples}, seed: {seed}");
    println!();

    let all_miners: Vec<String> = {
        let mut set: BTreeSet<String> = BTreeSet::new();
        for row in rows {
            set.insert(row.miner_slug.clone());
        }
        set.into_iter().collect()
    };

    println!("--- all three miners together ---");
    print_one(rows, Scorer::Ours, &all_miners, resamples, seed);
    print_one(rows, Scorer::Reference, &all_miners, resamples, seed);
    println!();

    println!("--- pairwise, which uses more clusters per comparison ---");
    for left in 0..all_miners.len() {
        for right in (left + 1)..all_miners.len() {
            let pair = vec![all_miners[left].clone(), all_miners[right].clone()];
            println!("  pair: {} vs {}", pair[0], pair[1]);
            print_one(rows, Scorer::Ours, &pair, resamples, seed);
            print_one(rows, Scorer::Reference, &pair, resamples, seed);
        }
    }
}
