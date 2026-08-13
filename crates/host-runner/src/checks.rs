//! This module has the determinism checks. Each function drives a
//! [`ScriptInstance`] and reports whether the result was stable.
//!
//! Every check in this module crosses the wasm boundary: each one
//! calls `rank_answer` through `wasmtime`, the same way a validator
//! would. An earlier version of this module also had two native,
//! non-wasm checks, for a log loss metric and a batch scoring path.
//! Neither the log loss metric nor the batch path is part of the
//! published ABI -- `rank_answer` is the one scoring export -- so
//! those two checks are gone, not ported.

use std::path::Path;

use anyhow::{Context, Result};

use crate::instance::ScriptInstance;

/// This is the result of a repeated `rank_answer` stability check.
pub struct RepeatReport {
    /// The number of calls made.
    pub run_count: u32,
    /// The single `f32` bit pattern, as hex text, if all calls agreed.
    pub bits_hex: String,
    /// The number of distinct bit patterns seen. This must be 1 to pass.
    pub distinct_count: usize,
    /// All distinct bit patterns seen, as hex text. Normally this has one
    /// entry.
    pub distinct_bits_hex: Vec<String>,
    /// True if the check passed, that is, `distinct_count == 1`.
    pub pass: bool,
}

/// This calls `rank_answer` 1000 times on one fixed input and checks
/// that every call returns the same `f32` bit pattern.
///
/// This is a wasm boundary check: every call goes through `wasmtime`.
pub fn check_rank_answer_repeat_stability(
    instance: &mut ScriptInstance,
    question: &[u8],
    gt: &[u8],
    ma: &[u8],
) -> Result<RepeatReport> {
    let mut bits_list = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let value = instance
            .rank_answer(question, gt, ma)
            .context("a call to 'rank_answer' failed during the 1000-run stability check")?;
        bits_list.push(value.to_bits());
    }
    Ok(summarize_bits_32(1000, bits_list))
}

/// This turns a list of `f32` bit patterns into a [`RepeatReport`],
/// with an 8 digit hex format.
fn summarize_bits_32(run_count: u32, bits_list: Vec<u32>) -> RepeatReport {
    let mut distinct: Vec<u32> = bits_list.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let pass = distinct.len() == 1;
    RepeatReport {
        run_count,
        bits_hex: format!("0x{:08x}", bits_list[0]),
        distinct_count: distinct.len(),
        distinct_bits_hex: distinct.iter().map(|b| format!("0x{b:08x}")).collect(),
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

/// This runs one `rank_answer` call on a fresh instance and compares
/// it, bit for bit, to a call on an instance that has already run
/// other calls.
///
/// This check finds state that leaks between calls or between
/// instances. A validator must get the same score no matter how many
/// calls a `ScriptInstance` served before this one. This is a wasm
/// boundary check: both calls go through `wasmtime`.
pub fn check_fresh_vs_reused(
    wasm_path: &Path,
    reused_instance: &mut ScriptInstance,
    question: &[u8],
    gt: &[u8],
    ma: &[u8],
) -> Result<FreshVsReusedReport> {
    let mut fresh_instance =
        ScriptInstance::load(wasm_path).context("cannot load a fresh instance for comparison")?;
    let fresh_value = fresh_instance
        .rank_answer(question, gt, ma)
        .context("a call to 'rank_answer' failed on the fresh instance")?;
    let reused_value = reused_instance
        .rank_answer(question, gt, ma)
        .context("a call to 'rank_answer' failed on the reused instance")?;
    let fresh_bits = fresh_value.to_bits();
    let reused_bits = reused_value.to_bits();
    Ok(FreshVsReusedReport {
        fresh_bits_hex: format!("0x{fresh_bits:08x}"),
        reused_bits_hex: format!("0x{reused_bits:08x}"),
        pass: fresh_bits == reused_bits,
    })
}
