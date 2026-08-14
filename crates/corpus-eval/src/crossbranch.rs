//! This module holds the cross-branch adversarial cases.
//!
//! The wave 3 suite attacks inside one scoring branch at a time. The
//! stronger attack crosses branches: the miner sends something that
//! makes the scorer LEAVE the numeric branch, and then farms the text
//! branch, which has a much softer floor.
//!
//! The worked example is a ground truth of "192.43 USD" and an answer
//! of "USD". The answer holds no number, so the scorer leaves the
//! numeric path and compares tokens instead. The union of {usd} and
//! {192.43, usd} is 2 and the intersection is 1, so a constant string
//! that carries no information scores 0.5.

use eval_script::score::score_answer;

use crate::baseline::baseline_score;

/// One cross-branch case.
pub struct CrossBranchCase {
    /// The group this case belongs to, for the report.
    pub group: &'static str,
    /// The ground truth text.
    pub ground_truth: &'static str,
    /// The miner answer text.
    pub answer: &'static str,
}

/// This function gives every cross-branch case.
///
/// The list covers the five shapes the wave 4 brief names, plus the
/// honest reference rows that the attacks must not beat.
pub fn cases() -> Vec<CrossBranchCase> {
    vec![
        // Group 1: the answer is the unit only.
        CrossBranchCase {
            group: "unit only",
            ground_truth: "192.43 USD",
            answer: "USD",
        },
        CrossBranchCase {
            group: "unit only",
            ground_truth: "34.7 C",
            answer: "C",
        },
        CrossBranchCase {
            group: "unit only",
            ground_truth: "307.85 K",
            answer: "K",
        },
        CrossBranchCase {
            group: "unit only",
            ground_truth: "15 %",
            answer: "%",
        },
        CrossBranchCase {
            group: "unit only",
            ground_truth: "12 gwei",
            answer: "gwei",
        },
        // Group 2: the answer is a subset of the non-numeric tokens.
        CrossBranchCase {
            group: "non-numeric subset",
            ground_truth: "The temperature was 28.9 C.",
            answer: "temperature",
        },
        CrossBranchCase {
            group: "non-numeric subset",
            ground_truth: "The temperature was 28.9 C.",
            answer: "the temperature was C",
        },
        // Group 3: the currency code only, against a symbol form.
        CrossBranchCase {
            group: "currency code only",
            ground_truth: "$192.43",
            answer: "USD",
        },
        // Group 4: the unit repeated many times.
        CrossBranchCase {
            group: "unit repeated",
            ground_truth: "192.43 USD",
            answer: "USD USD USD USD USD USD USD USD",
        },
        // Group 5: ground truth is text, the answer is a number inside
        // it.
        CrossBranchCase {
            group: "number inside text truth",
            ground_truth: "The temperature was 28.9 C.",
            answer: "28.9",
        },
        CrossBranchCase {
            group: "number inside text truth",
            ground_truth: "The temperature was 28.9 C.",
            answer: "28.9 C",
        },
        // The honest rows. Every attack above must stay below these.
        CrossBranchCase {
            group: "honest, exact",
            ground_truth: "192.43 USD",
            answer: "192.43 USD",
        },
        CrossBranchCase {
            group: "honest, 10 percent out",
            ground_truth: "192.43 USD",
            answer: "211.67 USD",
        },
        CrossBranchCase {
            group: "honest, 10 percent out",
            ground_truth: "34.7 C",
            answer: "38.2 C",
        },
    ]
}

/// One scored cross-branch row.
pub struct CrossBranchResult {
    /// The group this case belongs to.
    pub group: &'static str,
    /// The ground truth text.
    pub ground_truth: &'static str,
    /// The miner answer text.
    pub answer: &'static str,
    /// Our score.
    pub ours: f64,
    /// The baseline score.
    pub baseline: f64,
}

/// This function scores every cross-branch case.
pub fn run() -> Vec<CrossBranchResult> {
    cases()
        .into_iter()
        .map(|case| CrossBranchResult {
            group: case.group,
            ground_truth: case.ground_truth,
            answer: case.answer,
            ours: score_answer("", case.ground_truth, case.answer),
            baseline: baseline_score(case.ground_truth, case.answer),
        })
        .collect()
}

/// This function prints the cross-branch table.
pub fn print_table() {
    let results = run();
    println!(
        "{:<26} {:<30} {:<26} {:>10} {:>10}",
        "group", "ground truth", "answer", "ours", "baseline"
    );
    for row in &results {
        println!(
            "{:<26} {:<30} {:<26} {:>10.6} {:>10.4}",
            row.group,
            truncate(row.ground_truth, 29),
            truncate(row.answer, 25),
            row.ours,
            row.baseline
        );
    }

    // The comparison that matters: does any attack beat an honest
    // miner that is 10 percent out?
    let honest = results
        .iter()
        .filter(|row| row.group == "honest, 10 percent out")
        .map(|row| row.ours)
        .fold(f64::INFINITY, f64::min);
    println!();
    println!("honest miner 10 percent out scores: {honest:.6}");
    let mut beats = 0usize;
    for row in &results {
        if row.group.starts_with("honest") {
            continue;
        }
        if row.ours > honest {
            println!(
                "  BEATS HONEST: {:?} -> {:?} scores {:.6}",
                row.ground_truth, row.answer, row.ours
            );
            beats += 1;
        }
    }
    if beats == 0 {
        println!("  no attack beats the honest miner");
    }
}

/// This function cuts a text to a length, for a table cell.
fn truncate(text: &str, width: usize) -> String {
    if text.len() <= width {
        text.to_string()
    } else {
        let mut cut = text[..width.saturating_sub(1)].to_string();
        cut.push('~');
        cut
    }
}

/// This function prints how the three ground-truth renderings score.
///
/// The real ground-truth format is undisclosed, so a score that
/// changes with the rendering is a defect, not a detail.
pub fn print_renderings() {
    let bare = "28.9";
    let prose = "The temperature was 28.9 C.";
    let json = "{\"temperature_2m\":28.9,\"time\":\"2026-08-10T12:00\"}";
    let answers = [
        "28.9",
        "28.9 C",
        "302.05 K",
        "28.5",
        "35.0",
        "the temperature was C",
        "temperature",
        "2026",
        "12",
    ];
    println!(
        "{:<24} {:>12} {:>12} {:>12}",
        "answer", "gt_bare", "gt_prose", "gt_json"
    );
    for answer in answers {
        println!(
            "{:<24} {:>12.6} {:>12.6} {:>12.6}",
            format!("{answer:?}"),
            score_answer("", bare, answer),
            score_answer("", prose, answer),
            score_answer("", json, answer),
        );
    }
}

/// This function prints the headline numeric separation table.
///
/// The table is the clearest single result in the submission. The
/// baseline gives 0.0 to an answer one cent out AND to an answer a
/// million out. It cannot tell them apart. It also fails two answers
/// that hold the SAME number written a different way.
pub fn print_separation() {
    let truth = "192.43";
    let answers = [
        "192.43",
        "192.44",
        "$192.43",
        "192.430",
        "192.43 USD",
        "999999.99",
    ];
    println!("=== NUMERIC SEPARATION, GROUND TRUTH \"192.43\" ===");
    println!("{:<14} {:>18} {:>12}   note", "answer", "ours", "baseline");
    for answer in answers {
        let ours = score_answer("", truth, answer);
        let base = baseline_score(truth, answer);
        let note = match answer {
            "192.44" => "one cent out",
            "999999.99" => "a million out",
            "$192.43" | "192.43 USD" => "same number, unit added",
            "192.430" => "same number, trailing zero",
            _ => "exact",
        };
        println!(
            "{:<14} {ours:>18.10} {base:>12.4}   {note}",
            format!("{answer:?}")
        );
    }
}
