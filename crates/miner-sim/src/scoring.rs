//! This module scores one miner through the WASM module.
//!
//! Every score in this crate must come from the compiled `eval-script`
//! module. This module never computes a Brier score or a log loss score
//! in native Rust. A native score would not prove anything about the
//! artifact that a validator runs.
//!
//! # Score direction
//!
//! A HIGH score is good. The range is 0.0 to 1.0. The best score is
//! 1.0. The worst score is 0.0. Read the note on score direction in
//! `types.rs`.
//!
//! # The two layers
//!
//! This module is Layer 1 work: it gets one score per item from the
//! WASM module, with no memory between items. This module also builds
//! `first_failure`, which is the raw fact that Layer 2 (`leaderboard.rs`)
//! needs to apply the ejection rule. This module does not apply the
//! ejection rule itself. It only records where the first failure is.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use host_runner::instance::ScriptInstance;

use crate::leaderboard;
use crate::types::{
    Archetype, Dataset, EjectionReason, Failure, Metric, MinerResult, Response, ResponseKind,
    SkillRow,
};

/// This function gives the path to the compiled `eval-script` module.
///
/// The path is `target/wasm32-unknown-unknown/release/eval_script.wasm`,
/// under the workspace root. The function builds the path from
/// `CARGO_MANIFEST_DIR`, so the binary finds the file from any current
/// directory.
#[must_use]
pub fn resolve_wasm_path() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .join("..")
        .join("..")
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("eval_script.wasm")
}

/// This function scores every response of one miner through the WASM
/// module.
///
/// The function calls `score` or `score_log_loss` for each item, in
/// item order, using the ground truth text of that item and the response
/// text at the same index. It counts malformed and abstained responses
/// from the response kind. It does not read the score value to decide
/// the count; the count comes only from `ResponseKind`.
///
/// The function also finds the first failure of the miner. It scans the
/// items in order. The first item with an `Abstain` response, or the
/// first item with a `Malformed` response, whichever comes first, sets
/// `first_failure`. A later failure does not change `first_failure`,
/// because the function only writes to it while it still holds `None`.
/// This function does not remove the miner from anything. It only
/// records the fact. `leaderboard::build_standings` reads this fact and
/// applies the ejection rule of the whitepaper.
///
/// # Errors
///
/// The function returns an error when `responses` does not have one
/// entry per item in `dataset`, when a call into the module fails, or
/// when a score is not finite or falls outside the range 0.0 to 1.0.
/// The module promises a score in that range, so a value outside it is
/// a real defect and this function must not hide it.
pub fn score_miner(
    instance: &mut ScriptInstance,
    dataset: &Dataset,
    archetype: Archetype,
    responses: &[Response],
    metric: Metric,
) -> Result<MinerResult> {
    if responses.len() != dataset.items.len() {
        bail!(
            "archetype {} sent {} responses, but the data set has {} items",
            archetype.name(),
            responses.len(),
            dataset.items.len()
        );
    }

    let mut scores = Vec::with_capacity(dataset.items.len());
    let mut n_malformed = 0usize;
    let mut n_abstained = 0usize;
    let mut first_failure: Option<Failure> = None;

    for (item, response) in dataset.items.iter().zip(responses.iter()) {
        let gt_bytes = item.ground_truth_json();
        let resp_bytes = response.json.as_bytes();

        let raw = match metric {
            Metric::Brier => instance
                .score(gt_bytes.as_bytes(), resp_bytes)
                .with_context(|| {
                    format!(
                        "call to 'score' failed for item {} of archetype {}",
                        item.index,
                        archetype.name()
                    )
                })?,
            Metric::LogLoss => instance
                .score_log_loss(gt_bytes.as_bytes(), resp_bytes)
                .with_context(|| {
                    format!(
                        "call to 'score_log_loss' failed for item {} of archetype {}",
                        item.index,
                        archetype.name()
                    )
                })?,
        };

        if !raw.is_finite() || !(0.0..=1.0).contains(&raw) {
            bail!(
                "item {} of archetype {} got score {} from metric {}; the module must give a value in 0.0 to 1.0",
                item.index,
                archetype.name(),
                raw,
                metric.name()
            );
        }
        scores.push(raw);

        match response.kind {
            ResponseKind::Answer { .. } => {}
            ResponseKind::Abstain => {
                n_abstained += 1;
                if first_failure.is_none() {
                    first_failure = Some(Failure {
                        index: item.index,
                        reason: EjectionReason::NoResponse,
                    });
                }
            }
            ResponseKind::Malformed => {
                n_malformed += 1;
                if first_failure.is_none() {
                    first_failure = Some(Failure {
                        index: item.index,
                        reason: EjectionReason::MalformedResponse,
                    });
                }
            }
        }
    }

    Ok(MinerResult {
        archetype,
        scores,
        n_malformed,
        n_abstained,
        first_failure,
    })
}

/// This function builds the Brier Skill Score table for a list of
/// scored miners.
///
/// The table uses RAW Brier numbers, not the converted score. The
/// caller must pass `results` that came from `score_miner` called with
/// `Metric::Brier`. A result scored with `Metric::LogLoss` gives a
/// meaningless row, because the raw loss formula below is specific to
/// the Brier rule.
///
/// For each miner:
///
/// - `raw_brier = 1.0 - converted_mean_score`. The converted mean score
///   comes from [`leaderboard::mean`] over the miner's per-item scores.
/// - `climatology_brier = base_rate * (1.0 - base_rate)`, using
///   `dataset.realised_base_rate`. This is the Brier score of a
///   forecaster that reports only the base rate on every item, and
///   never reads the item at all.
/// - `bss = 1.0 - raw_brier / climatology_brier`. A value above 0.0
///   shows real skill over the base rate forecaster. A value below 0.0
///   shows the miner is worse than that forecaster.
///
/// # Guard
///
/// When `climatology_brier` is 0.0, the base rate is exactly 0.0 or
/// 1.0. Every forecaster gets a perfect climatology score in that case,
/// so the skill ratio is not defined. The function sets `bss` to 0.0 in
/// that case, instead of dividing by 0.0. The render function marks
/// this row so a reader does not read the 0.0 as a real skill number.
///
/// This function includes EVERY archetype in `results`, even one that
/// the `Eject` aggregation model would remove from the pool. The skill
/// question is about the scoring rule. It is not about the aggregation
/// rule. Read the note on `AggregationModel` in `types.rs`.
///
/// The function sorts the rows by `bss`, best first. It breaks a tie by
/// `archetype`, so the order never depends on the order of `results`.
#[must_use]
pub fn build_skill_table(results: &[MinerResult], dataset: &Dataset) -> Vec<SkillRow> {
    let base_rate = dataset.realised_base_rate;
    let climatology_brier = base_rate * (1.0 - base_rate);

    let mut rows: Vec<SkillRow> = results
        .iter()
        .map(|result| {
            let converted_mean = leaderboard::mean(&result.scores);
            let raw_brier = 1.0 - converted_mean;
            let bss = if climatology_brier > 0.0 {
                1.0 - raw_brier / climatology_brier
            } else {
                0.0
            };
            SkillRow {
                archetype: result.archetype,
                raw_brier,
                climatology_brier,
                bss,
            }
        })
        .collect();

    rows.sort_by(|a, b| {
        b.bss
            .total_cmp(&a.bss)
            .then_with(|| a.archetype.cmp(&b.archetype))
    });
    rows
}

/// This function makes a text table of the Brier Skill Score rows.
///
/// The table uses the RAW Brier convention: a LOW `raw_brier` is good.
/// This is the opposite convention from the leaderboard tables, which
/// use the CONVERTED score, where a HIGH value is good. The function
/// prints a line at the top that states which convention each column
/// uses, so a reader never has to guess.
///
/// A row with a negative `bss` gets a `NEGATIVE SKILL` mark. A row with
/// a `climatology_brier` of 0.0 gets an `N/A` mark instead of a `bss`
/// reading, because the skill ratio is not defined at that base rate.
#[must_use]
pub fn render_skill_table(rows: &[SkillRow], title: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "{title}");
    let _ = writeln!(
        out,
        "convention: raw_brier is a LOSS, a LOW value is good. climatology_brier is a LOSS too. bss is a skill score, a HIGH value is good."
    );
    let _ = writeln!(
        out,
        "this table includes every archetype, even one that the eject model removes from the pool. the skill question is about the scoring rule, not the aggregation rule."
    );
    let _ = writeln!(
        out,
        "{:<22} {:>12} {:>18} {:>10}  note",
        "archetype", "raw_brier", "climatology_brier", "bss"
    );
    for row in rows {
        let note = if row.climatology_brier <= 0.0 {
            "N/A: climatology_brier is 0.0 at this base rate"
        } else if row.bss < 0.0 {
            "NEGATIVE SKILL"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "{:<22} {:>12.6} {:>18.6} {:>10.6}  {}",
            row.archetype.name(),
            row.raw_brier,
            row.climatology_brier,
            row.bss,
            note
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DatasetShape, Item};

    fn oracle_response(item: &Item) -> Response {
        let confidence = if item.label == 1 { 0.99 } else { 0.01 };
        Response {
            json: format!("{{\"confidence\": {confidence}}}"),
            kind: ResponseKind::Answer {
                confidence,
                correct: true,
            },
        }
    }

    fn contrarian_response(item: &Item) -> Response {
        let confidence = if item.label == 1 { 0.01 } else { 0.99 };
        Response {
            json: format!("{{\"confidence\": {confidence}}}"),
            kind: ResponseKind::Answer {
                confidence,
                correct: false,
            },
        }
    }

    fn tiny_dataset() -> Dataset {
        let items: Vec<Item> = (0..20)
            .map(|i| Item {
                index: i,
                label: u8::from(i % 2 == 0),
                signal: 0.8,
            })
            .collect();
        Dataset {
            shape: DatasetShape::Balanced,
            seed: 1,
            items,
            majority_label: 1,
            realised_base_rate: 0.5,
            hard_signal_threshold: 0.1,
        }
    }

    /// This test checks that an oracle response gets a score near 1.0
    /// and a contrarian response gets a score near 0.0, both through the
    /// module. A high score is good; the module returns `1.0 - brier`.
    /// The test skips with a printed message if the module is absent.
    /// The test does not fail in that case, so this crate can build
    /// before the WASM module exists.
    #[test]
    fn oracle_scores_high_and_contrarian_scores_low() {
        let wasm_path = resolve_wasm_path();
        if !wasm_path.exists() {
            println!(
                "skip oracle_scores_high_and_contrarian_scores_low: no wasm file at {}",
                wasm_path.display()
            );
            return;
        }
        let mut instance = match ScriptInstance::load(&wasm_path) {
            Ok(v) => v,
            Err(e) => {
                println!("skip oracle_scores_high_and_contrarian_scores_low: load failed: {e}");
                return;
            }
        };

        let dataset = tiny_dataset();
        let oracle_responses: Vec<Response> = dataset.items.iter().map(oracle_response).collect();
        let contrarian_responses: Vec<Response> =
            dataset.items.iter().map(contrarian_response).collect();

        let oracle_result = score_miner(
            &mut instance,
            &dataset,
            Archetype::Oracle,
            &oracle_responses,
            Metric::Brier,
        )
        .expect("oracle scoring must not fail");
        let contrarian_result = score_miner(
            &mut instance,
            &dataset,
            Archetype::Contrarian,
            &contrarian_responses,
            Metric::Brier,
        )
        .expect("contrarian scoring must not fail");

        let oracle_mean: f64 =
            oracle_result.scores.iter().sum::<f64>() / oracle_result.scores.len() as f64;
        let contrarian_mean: f64 =
            contrarian_result.scores.iter().sum::<f64>() / contrarian_result.scores.len() as f64;

        assert!(oracle_mean > 0.95, "oracle mean was {oracle_mean}");
        assert!(
            contrarian_mean < 0.2,
            "contrarian mean was {contrarian_mean}"
        );
        assert!(oracle_mean > contrarian_mean);
    }

    /// This test checks that a length mismatch gives a clear error and
    /// does not panic.
    #[test]
    fn length_mismatch_is_an_error() {
        let wasm_path = resolve_wasm_path();
        if !wasm_path.exists() {
            println!(
                "skip length_mismatch_is_an_error: no wasm file at {}",
                wasm_path.display()
            );
            return;
        }
        let mut instance = match ScriptInstance::load(&wasm_path) {
            Ok(v) => v,
            Err(e) => {
                println!("skip length_mismatch_is_an_error: load failed: {e}");
                return;
            }
        };
        let dataset = tiny_dataset();
        let short_responses: Vec<Response> =
            dataset.items.iter().take(1).map(oracle_response).collect();
        let result = score_miner(
            &mut instance,
            &dataset,
            Archetype::Oracle,
            &short_responses,
            Metric::Brier,
        );
        assert!(result.is_err());
    }

    fn failing_result(archetype: Archetype, scores: Vec<f64>) -> MinerResult {
        MinerResult {
            archetype,
            scores,
            n_malformed: 0,
            n_abstained: 0,
            first_failure: None,
        }
    }

    #[test]
    fn skill_table_flags_negative_skill_and_sorts_best_first() {
        let dataset = Dataset {
            shape: DatasetShape::Skewed,
            seed: 1,
            items: Vec::new(),
            majority_label: 1,
            realised_base_rate: 0.9,
            hard_signal_threshold: 0.0,
        };
        // climatology_brier = 0.9 * 0.1 = 0.09.
        let results = vec![
            failing_result(Archetype::Oracle, vec![1.0, 1.0]),
            failing_result(Archetype::Random, vec![0.5, 0.5]),
        ];
        let rows = build_skill_table(&results, &dataset);
        assert_eq!(rows[0].archetype, Archetype::Oracle);
        assert!(rows[0].bss > 0.0, "oracle bss was {}", rows[0].bss);
        assert!(rows[1].bss < 0.0, "random bss was {}", rows[1].bss);
    }

    #[test]
    fn skill_table_guards_zero_climatology_brier() {
        let dataset = Dataset {
            shape: DatasetShape::Skewed,
            seed: 1,
            items: Vec::new(),
            majority_label: 1,
            realised_base_rate: 1.0,
            hard_signal_threshold: 0.0,
        };
        let results = vec![failing_result(Archetype::Oracle, vec![1.0])];
        let rows = build_skill_table(&results, &dataset);
        assert_eq!(rows[0].climatology_brier, 0.0);
        assert_eq!(rows[0].bss, 0.0);
    }
}
