//! This module scores the adversarial table through the REAL wasm
//! modules, not through the native copy of the baseline.
//!
//! ## Why this module exists
//!
//! The first version of the evaluation built its `reference` column
//! from `baseline::baseline_score`, a native Rust copy of the
//! protocol's published `word_overlap`. The copy has tests against the
//! published source, but a reviewer is right to distrust the sentence
//! "we reimplemented their scorer and ours beats it". The number must
//! come from the compiled module the protocol ships.
//!
//! So this module sends every adversarial case through the SAME path
//! the corpus columns already use: `corpus-eval` writes prepared rows,
//! `tools/wazero-runner` scores them with both `.wasm` modules under
//! wazero, and `corpus-eval` reduces the result. No second harness, no
//! special case.
//!
//! ## Why all three renderings hold the same text
//!
//! The corpus harness scores three ground-truth renderings per row,
//! because the real ground-truth format is undisclosed. An adversarial
//! case is one literal pair, so there is nothing to re-render: all
//! three fields carry the same text. That is not waste. Three
//! identical inputs through the same module must give three identical
//! scores, so the readback checks it and fails loudly if the harness
//! is not deterministic.
//!
//! ## The native copy after this change
//!
//! `baseline::baseline_score` stays, as a TEST ORACLE only. The
//! readback compares it against the wasm result on every row and
//! reports any disagreement. It is no longer the source of a published
//! number.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use eval_script::score::score_answer;
use serde::Serialize;

use crate::baseline::baseline_score;
use crate::stats::ScoredRow;

/// One adversarial case.
pub struct Case {
    /// The row label in the report.
    pub label: &'static str,
    /// Which table this row belongs to.
    pub table: Table,
    /// The question text. Most cases send an empty question, because
    /// the question is advisory. The two copy-the-question cases need
    /// a real one.
    pub question: String,
    /// The ground truth text.
    pub ground_truth: String,
    /// The miner answer text.
    pub answer: String,
}

/// Which published table a case belongs to.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Table {
    /// The headline numeric separation table in section 1.
    Separation,
    /// The main strategy table.
    Strategy,
    /// The cross-branch farming table.
    CrossBranch,
}

impl Table {
    /// This function gives the heading for a table.
    fn heading(self) -> &'static str {
        match self {
            Table::Separation => "NUMERIC SEPARATION, GROUND TRUTH \"192.43\"",
            Table::Strategy => "STRATEGY TABLE",
            Table::CrossBranch => "CROSS-BRANCH TABLE",
        }
    }

    /// This function gives how many decimal places our column shows.
    ///
    /// The separation table needs ten, because its whole point is that
    /// two wrong answers differ by ten orders of magnitude. Six is
    /// enough everywhere else.
    fn our_decimals(self) -> usize {
        match self {
            Table::Separation => 10,
            _ => 6,
        }
    }
}

/// This function builds the answer used by the padding case.
///
/// The padding repeats ONE word. A token set removes the duplicates,
/// so the padding adds exactly one distinct token however long it
/// gets. Padding with DISTINCT words scores far lower, which the test
/// `the_score_does_not_grow_with_answer_length` covers.
fn repeated_word_padding() -> String {
    let mut padded = String::from("malicious");
    for _ in 0..200 {
        padded.push_str(" filler");
    }
    padded
}

/// This function gives every adversarial case, in report order.
///
/// The list is the single source of the published table. A row that is
/// not here is not in the report.
pub fn cases() -> Vec<Case> {
    let junk_question = "[direct] 207 -> /price";
    let plain_question = "what is the current temperature in tokyo";

    // The section 1 headline table. The reference gives the same
    // 0.0000 to an answer one cent out and to an answer a million out.
    let separation = [
        ("exact", "192.43".to_string()),
        ("one cent out", "192.44".to_string()),
        ("same number, symbol added", "$192.43".to_string()),
        ("same number, trailing zero", "192.430".to_string()),
        ("same number, code added", "192.43 USD".to_string()),
        ("a million out", "999999.99".to_string()),
    ];

    let strategy = [
        ("constant word", "", "192.43", "yes".to_string()),
        ("most common number", "", "192.43", "100".to_string()),
        (
            "subset of ground truth",
            "",
            "high risk malicious binary",
            "malicious".to_string(),
        ),
        ("empty", "", "192.43", String::new()),
        (
            "control characters",
            "",
            "192.43",
            "\u{0}\u{1}\u{2}".to_string(),
        ),
        (
            "long padded answer",
            "",
            "malicious",
            repeated_word_padding(),
        ),
        (
            "many candidate numbers",
            "",
            "192.43",
            "1 2 5 10 20 50 100 150 192.43 200 250 300 500 1000".to_string(),
        ),
        (
            "unit spoof, K value as C",
            "",
            "34.7 C",
            "307.85 C".to_string(),
        ),
        (
            "precision spam",
            "",
            "192.43",
            "192.4300000000001".to_string(),
        ),
        ("hedge word", "", "42", "about 42".to_string()),
        ("hedged range", "", "35", "34 to 36".to_string()),
        ("negation", "", "malicious", "not malicious".to_string()),
        (
            "double negation",
            "",
            "malicious",
            "not not malicious".to_string(),
        ),
        ("one common token", "", "is malicious", "is".to_string()),
        (
            "copy question back",
            plain_question,
            "34.7 C",
            plain_question.to_string(),
        ),
        (
            "copy junk question back",
            junk_question,
            "192.43",
            junk_question.to_string(),
        ),
    ];

    let cross_branch = [
        ("unit only, USD", "192.43 USD", "USD".to_string()),
        ("unit only, C", "34.7 C", "C".to_string()),
        ("unit only, gwei", "12 gwei", "gwei".to_string()),
        (
            "unit repeated",
            "192.43 USD",
            "USD USD USD USD USD USD USD USD".to_string(),
        ),
        (
            "prose words, no number",
            "The temperature was 28.9 C.",
            "the temperature was C".to_string(),
        ),
        (
            "one prose token",
            "The temperature was 28.9 C.",
            "temperature".to_string(),
        ),
    ];

    let mut out = Vec::new();
    for (label, answer) in separation {
        out.push(Case {
            label,
            table: Table::Separation,
            question: String::new(),
            ground_truth: "192.43".to_string(),
            answer,
        });
    }
    for (label, question, ground_truth, answer) in strategy {
        out.push(Case {
            label,
            table: Table::Strategy,
            question: question.to_string(),
            ground_truth: ground_truth.to_string(),
            answer,
        });
    }
    for (label, ground_truth, answer) in cross_branch {
        out.push(Case {
            label,
            table: Table::CrossBranch,
            question: String::new(),
            ground_truth: ground_truth.to_string(),
            answer,
        });
    }
    out
}

/// One prepared row, in the shape the wazero corpus mode reads.
///
/// The field names match `EvalRow` in `tools/wazero-runner/corpus.go`
/// exactly. The numeric and miner fields carry filler, because the
/// harness passes them through without reading them.
#[derive(Serialize)]
struct PreparedRow<'a> {
    row_id: usize,
    question: &'a str,
    gt_bare: &'a str,
    gt_prose: &'a str,
    gt_json: &'a str,
    miner_value: &'a str,
    miner_slug: &'a str,
    intent: &'a str,
    valid_time: &'a str,
    actual_c: f64,
    miner_c: f64,
    cluster_id: &'a str,
}

/// This function writes the adversarial cases for the wazero runner.
///
/// `row_id` is the index into `cases()`, so the readback can put a
/// label back on each score.
pub fn emit(path: &Path) -> std::io::Result<usize> {
    let cases = cases();
    let mut writer = BufWriter::new(File::create(path)?);
    for (index, case) in cases.iter().enumerate() {
        let row = PreparedRow {
            row_id: index,
            question: &case.question,
            // All three renderings hold the same literal text. See the
            // module doc comment for why.
            gt_bare: &case.ground_truth,
            gt_prose: &case.ground_truth,
            gt_json: &case.ground_truth,
            miner_value: &case.answer,
            miner_slug: "adversarial",
            intent: "ADVERSARIAL",
            valid_time: "",
            actual_c: 0.0,
            miner_c: 0.0,
            cluster_id: "",
        };
        let line = serde_json::to_string(&row).map_err(std::io::Error::other)?;
        writeln!(writer, "{line}")?;
    }
    writer.flush()?;
    Ok(cases.len())
}

/// The largest difference that counts as "the same number".
///
/// The wasm side returns an `f32`, and the native side computes in
/// `f64`. A row that agrees to `f32` precision is the same result. A
/// row that differs by more than this is a real disagreement and the
/// report must name it.
const AGREEMENT_EPSILON: f64 = 1e-6;

/// This function prints the adversarial table from the wasm scores.
///
/// The function reads the file the wazero runner wrote. It prints the
/// published table, and then a second table naming every row where the
/// compiled module and the native copy disagree.
pub fn print_report(scored: &[ScoredRow]) -> Result<(), String> {
    let cases = cases();
    if scored.len() != cases.len() {
        return Err(format!(
            "the score file has {} rows but there are {} cases; rerun the emit and the runner",
            scored.len(),
            cases.len()
        ));
    }

    // The three renderings hold the same text, so the three scores
    // must be the same. A difference means the harness is not
    // deterministic, and every number below is then suspect.
    for row in scored {
        let ours_spread = (row.ours_bare - row.ours_prose)
            .abs()
            .max((row.ours_bare - row.ours_json).abs());
        let ref_spread = (row.ref_bare - row.ref_prose)
            .abs()
            .max((row.ref_bare - row.ref_json).abs());
        if ours_spread > 0.0 || ref_spread > 0.0 {
            return Err(format!(
                "row {} scored three identical inputs differently: ours spread {ours_spread}, \
                 reference spread {ref_spread}",
                row.row_id
            ));
        }
    }

    let mut disagreements = Vec::new();

    for table in [Table::Separation, Table::Strategy, Table::CrossBranch] {
        println!();
        println!(
            "=== {}, SCORED BY THE COMPILED MODULES ===",
            table.heading()
        );
        println!(
            "{:<28} {:<30} {:<26} {:>14} {:>10}",
            "case", "ground truth", "answer", "ours", "reference"
        );
        for (index, case) in cases.iter().enumerate() {
            if case.table != table {
                continue;
            }
            let row = &scored[index];
            println!(
                "{:<28} {:<30} {:<26} {:>14.*} {:>10.4}",
                case.label,
                display_cell(&case.ground_truth, 29),
                display_cell(&case.answer, 25),
                table.our_decimals(),
                row.ours_bare,
                row.ref_bare
            );

            let native_reference = baseline_score(&case.ground_truth, &case.answer);
            let native_ours = score_answer(&case.question, &case.ground_truth, &case.answer);
            let reference_delta = (native_reference - row.ref_bare).abs();
            let ours_delta = (native_ours - row.ours_bare).abs();
            if reference_delta > AGREEMENT_EPSILON || ours_delta > AGREEMENT_EPSILON {
                disagreements.push((
                    case.label,
                    row.ref_bare,
                    native_reference,
                    row.ours_bare,
                    native_ours,
                ));
            }
        }
    }

    println!();
    println!("=== COMPILED MODULE VERSUS NATIVE COPY ===");
    if disagreements.is_empty() {
        println!(
            "all {} rows agree to within {AGREEMENT_EPSILON:e}",
            cases.len()
        );
        println!("the native word_overlap copy is faithful to the shipped module");
    } else {
        println!(
            "{:<26} {:>12} {:>12} {:>12} {:>12}",
            "strategy", "ref (wasm)", "ref (native)", "ours (wasm)", "ours (native)"
        );
        for (label, ref_wasm, ref_native, ours_wasm, ours_native) in &disagreements {
            println!("{label:<26} {ref_wasm:>12.6} {ref_native:>12.6} {ours_wasm:>12.6} {ours_native:>12.6}");
        }
    }

    Ok(())
}

/// This function makes a table cell out of a case string.
///
/// The function shows control characters as escapes, so the table
/// stays one row per case, and it cuts a long string with a tilde.
fn display_cell(text: &str, width: usize) -> String {
    if text.is_empty() {
        return "(empty)".to_string();
    }
    let mut shown = String::new();
    for character in text.chars() {
        match character {
            '\u{0}'..='\u{1f}' => shown.push_str(&format!("\\{}", character as u32)),
            _ => shown.push(character),
        }
    }
    if shown.chars().count() <= width {
        return shown;
    }
    let mut cut: String = shown.chars().take(width.saturating_sub(1)).collect();
    cut.push('~');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_has_a_label_and_the_labels_are_unique() {
        let cases = cases();
        for (index, case) in cases.iter().enumerate() {
            assert!(!case.label.is_empty(), "case {index} has no label");
            for other in cases.iter().skip(index + 1) {
                assert_ne!(case.label, other.label, "the label {} repeats", case.label);
            }
        }
    }

    #[test]
    fn every_table_has_its_published_row_count() {
        let cases = cases();
        assert_eq!(
            cases
                .iter()
                .filter(|c| c.table == Table::Separation)
                .count(),
            6
        );
        assert_eq!(
            cases.iter().filter(|c| c.table == Table::Strategy).count(),
            16
        );
        assert_eq!(
            cases
                .iter()
                .filter(|c| c.table == Table::CrossBranch)
                .count(),
            6
        );
    }

    #[test]
    fn the_separation_cases_hold_the_two_answers_the_reference_cannot_tell_apart() {
        // The headline claim of section 1 is that the reference gives
        // the same score to these two. If either case ever leaves the
        // list, that claim loses its evidence.
        let cases = cases();
        let separation: Vec<&str> = cases
            .iter()
            .filter(|c| c.table == Table::Separation)
            .map(|c| c.answer.as_str())
            .collect();
        assert!(separation.contains(&"192.44"), "the one-cent case is gone");
        assert!(
            separation.contains(&"999999.99"),
            "the a-million-out case is gone"
        );
    }

    #[test]
    fn the_padding_case_repeats_one_word() {
        // The point of this case is that a token SET removes the
        // duplicates. If the padding ever becomes distinct words, the
        // row measures a different thing.
        let padded = repeated_word_padding();
        let distinct: std::collections::BTreeSet<&str> = padded.split_whitespace().collect();
        assert_eq!(distinct.len(), 2, "the padding must add one distinct token");
    }

    #[test]
    fn a_control_character_cell_stays_on_one_line() {
        let cell = display_cell("\u{0}\u{1}\u{2}", 25);
        assert!(!cell.contains('\n'));
        assert_eq!(cell, "\\0\\1\\2");
    }

    #[test]
    fn an_empty_answer_is_shown_not_blank() {
        assert_eq!(display_cell("", 25), "(empty)");
    }
}
