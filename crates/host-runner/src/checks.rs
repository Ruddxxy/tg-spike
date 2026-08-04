//! This module has the determinism checks. Each function drives a
//! [`ScriptInstance`] and reports whether the result was stable.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use crate::instance::ScriptInstance;

/// This is the result of a repeated-call stability check.
pub struct RepeatReport {
    /// The number of calls made.
    pub run_count: u32,
    /// The single bit pattern, as hex text, if all calls agreed.
    pub bits_hex: String,
    /// The number of distinct bit patterns seen. This must be 1 to pass.
    pub distinct_count: usize,
    /// All distinct bit patterns seen, as hex text. Normally this has one
    /// entry.
    pub distinct_bits_hex: Vec<String>,
    /// True if the check passed, that is, `distinct_count == 1`.
    pub pass: bool,
}

/// This calls `score` 1000 times on one fixed input and checks that every
/// call returns the same bit pattern.
pub fn check_score_repeat_stability(
    instance: &mut ScriptInstance,
    gt: &[u8],
    resp: &[u8],
) -> Result<RepeatReport> {
    let mut bits_list = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let value = instance
            .score(gt, resp)
            .context("a call to 'score' failed during the 1000-run stability check")?;
        bits_list.push(value.to_bits());
    }
    Ok(summarize_bits(1000, bits_list))
}

/// This calls `score_log_loss` 1000 times on one fixed input and checks
/// that every call returns the same bit pattern.
pub fn check_score_log_loss_repeat_stability(
    instance: &mut ScriptInstance,
    gt: &[u8],
    resp: &[u8],
) -> Result<RepeatReport> {
    let mut bits_list = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let value = instance
            .score_log_loss(gt, resp)
            .context("a call to 'score_log_loss' failed during the 1000-run stability check")?;
        bits_list.push(value.to_bits());
    }
    Ok(summarize_bits(1000, bits_list))
}

/// This turns a list of bit patterns into a [`RepeatReport`].
fn summarize_bits(run_count: u32, bits_list: Vec<u64>) -> RepeatReport {
    let mut distinct: Vec<u64> = bits_list.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let pass = distinct.len() == 1;
    RepeatReport {
        run_count,
        bits_hex: format!("0x{:016x}", bits_list[0]),
        distinct_count: distinct.len(),
        distinct_bits_hex: distinct.iter().map(|b| format!("0x{b:016x}")).collect(),
        pass,
    }
}

/// This is the result of the fresh-instance-vs-reused-instance check.
pub struct FreshVsReusedReport {
    /// The bit pattern from a brand new instance.
    pub fresh_bits_hex: String,
    /// The bit pattern from the instance that ran the 1000-run check.
    pub reused_bits_hex: String,
    /// True if the two bit patterns match.
    pub pass: bool,
}

/// This runs one `score` call on a fresh instance and compares it, bit for
/// bit, to a call on an instance that has already run other calls.
///
/// This check finds state that leaks between calls or between instances.
/// A validator must get the same score no matter how many calls a
/// `ScriptInstance` served before this one.
pub fn check_fresh_vs_reused(
    wasm_path: &Path,
    reused_instance: &mut ScriptInstance,
    gt: &[u8],
    resp: &[u8],
) -> Result<FreshVsReusedReport> {
    let mut fresh_instance =
        ScriptInstance::load(wasm_path).context("cannot load a fresh instance for comparison")?;
    let fresh_value = fresh_instance
        .score(gt, resp)
        .context("a call to 'score' failed on the fresh instance")?;
    let reused_value = reused_instance
        .score(gt, resp)
        .context("a call to 'score' failed on the reused instance")?;
    let fresh_bits = fresh_value.to_bits();
    let reused_bits = reused_value.to_bits();
    Ok(FreshVsReusedReport {
        fresh_bits_hex: format!("0x{fresh_bits:016x}"),
        reused_bits_hex: format!("0x{reused_bits:016x}"),
        pass: fresh_bits == reused_bits,
    })
}

/// This is one ground-truth/response pair for the batch order check.
#[derive(Clone, Copy)]
pub struct BatchPair {
    pub label: u32,
    pub confidence: f64,
}

/// This builds the fixed set of 8 pairs used for the order invariance
/// check. The values are fixed in the source. Nothing here uses a random
/// number generator, so the check itself is repeatable.
pub fn fixed_batch_pairs() -> Vec<BatchPair> {
    vec![
        BatchPair {
            label: 1,
            confidence: 0.9,
        },
        BatchPair {
            label: 0,
            confidence: 0.1,
        },
        BatchPair {
            label: 1,
            confidence: 0.5,
        },
        BatchPair {
            label: 0,
            confidence: 0.5,
        },
        BatchPair {
            label: 1,
            confidence: 0.0,
        },
        BatchPair {
            label: 0,
            confidence: 1.0,
        },
        BatchPair {
            label: 1,
            confidence: 0.75,
        },
        BatchPair {
            label: 0,
            confidence: 0.25,
        },
    ]
}

/// This is one named, fixed ordering of the batch pairs.
pub struct NamedOrder {
    pub name: &'static str,
    pub order: Vec<usize>,
}

/// This gives the fixed orderings to test: original, reverse, a fixed
/// shuffle, and sorted by confidence. None of these use a random number
/// generator. Each order is a hardcoded list of indices into the 8 pairs
/// from [`fixed_batch_pairs`].
pub fn fixed_orderings(pairs: &[BatchPair]) -> Vec<NamedOrder> {
    let n = pairs.len();
    let original: Vec<usize> = (0..n).collect();
    let reverse: Vec<usize> = (0..n).rev().collect();
    // A fixed, hardcoded permutation. It is not derived from a random
    // number generator, so this order is the same on every run.
    let shuffle = vec![3, 0, 6, 1, 5, 2, 7, 4];

    // This uses `f64::total_cmp`, not `partial_cmp`, to match the
    // determinism discipline used for every float sort in this
    // workspace. `total_cmp` gives a total order and never needs an
    // `unwrap` or `expect` call.
    let mut sorted_by_confidence: Vec<usize> = (0..n).collect();
    sorted_by_confidence.sort_by(|&a, &b| pairs[a].confidence.total_cmp(&pairs[b].confidence));

    vec![
        NamedOrder {
            name: "original",
            order: original,
        },
        NamedOrder {
            name: "reverse",
            order: reverse,
        },
        NamedOrder {
            name: "fixed shuffle",
            order: shuffle,
        },
        NamedOrder {
            name: "sorted by confidence",
            order: sorted_by_confidence,
        },
    ]
}

/// This builds the `score_batch` JSON payload for one ordering of the
/// fixed pairs.
pub fn build_batch_json(pairs: &[BatchPair], order: &[usize]) -> Vec<u8> {
    let items: Vec<serde_json::Value> = order
        .iter()
        .map(|&i| {
            let pair = pairs[i];
            json!({
                "ground_truth": { "label": pair.label },
                "response": { "confidence": pair.confidence },
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::Value::Array(items))
        .expect("a batch built from plain numbers and strings always serializes")
}

/// This is the result of one ordering in the order invariance check.
pub struct OrderResult {
    pub name: &'static str,
    pub bits_hex: String,
    pub bits: u64,
}

/// This runs `score_batch` on the same 8 pairs in several fixed orders and
/// checks that every order gives the same bit pattern.
pub fn check_batch_order_invariance(
    instance: &mut ScriptInstance,
) -> Result<(Vec<OrderResult>, bool)> {
    let pairs = fixed_batch_pairs();
    let orderings = fixed_orderings(&pairs);

    let mut results = Vec::with_capacity(orderings.len());
    for named_order in &orderings {
        let batch_json = build_batch_json(&pairs, &named_order.order);
        let value = instance.score_batch(&batch_json).with_context(|| {
            format!(
                "call to 'score_batch' failed for order '{}'",
                named_order.name
            )
        })?;
        let bits = value.to_bits();
        results.push(OrderResult {
            name: named_order.name,
            bits_hex: format!("0x{bits:016x}"),
            bits,
        });
    }

    let first_bits = results[0].bits;
    let pass = results.iter().all(|r| r.bits == first_bits);
    Ok((results, pass))
}
