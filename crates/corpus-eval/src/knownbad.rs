//! This module reports the five known-bad corpus rows.
//!
//! ## What the evaluator can and cannot see
//!
//! Three of the five rows are wrong LOCATION or wrong TIME. The
//! evaluator never sees a location or a time. It sees one extracted
//! value against one ground-truth value. So it can only catch a wrong
//! place or a wrong date when that error produces a wrong NUMBER
//! against a correct truth.
//!
//! In this corpus it does not. The daemon-feed corpus joined the
//! archive actual at the coordinates and the valid time that the MINER
//! returned. So a miner
//! that answered for the wrong city was scored against the truth for
//! that wrong city, and the pair is self-consistent. The same holds
//! for a wrong date.
//!
//! This module measures that rather than claiming a catch. The result
//! is a limit of the corpus construction as much as of the scorer, and
//! the report states both.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::stats::{mean, ScoredRow};

/// One known-bad group.
pub struct KnownBad {
    /// The report name of the group.
    pub name: &'static str,
    /// What is wrong with the rows.
    pub defect: &'static str,
    /// Whether the evaluator could see the defect in principle.
    pub visible_to_evaluator: &'static str,
}

/// This function names the five known-bad groups.
pub fn groups() -> Vec<KnownBad> {
    vec![
        KnownBad {
            name: "alphavantage Ethereum",
            defect: "CRYPTO_PRICE for Ethereum answered with the equity ticker ETH at 17.94",
            visible_to_evaluator: "yes, a price of 17.94 against a real ETH price is far out",
        },
        KnownBad {
            name: "weatherapi Maringa",
            defect: "Maringa PR Brazil resolved to Brazil, Indiana, lat 39.52",
            visible_to_evaluator: "no, the truth was joined at the miner's own coordinates",
        },
        KnownBad {
            name: "openweathermap Miami climate",
            defect: "a question about October 2022 answered with an August 2026 forecast",
            visible_to_evaluator: "no, the truth was joined at the miner's own valid time",
        },
        KnownBad {
            name: "weatherapi Lisbon",
            defect: "result null while status said success",
            visible_to_evaluator: "yes, a null result gives no value at all",
        },
        KnownBad {
            name: "openweathermap moon",
            defect: "a question about a lunar impact resolved to a town named Moon in Iran",
            visible_to_evaluator: "no, the truth was joined at the miner's own coordinates",
        },
    ]
}

/// This function tells which group a question belongs to.
fn group_of(question: &str) -> Option<&'static str> {
    let lowered = question.to_lowercase();
    if question.contains("Maring") {
        Some("weatherapi Maringa")
    } else if question.contains("Miami") && question.contains("Climate") {
        Some("openweathermap Miami climate")
    } else if lowered.contains("moon") {
        Some("openweathermap moon")
    } else if lowered.contains("lisbon") {
        Some("weatherapi Lisbon")
    } else if lowered.contains("ethereum") {
        Some("alphavantage Ethereum")
    } else {
        None
    }
}

/// One row of the corpus, read only for its question and row number.
#[derive(serde::Deserialize)]
struct QuestionOnly {
    question: Option<String>,
}

/// This function finds which corpus rows belong to each group.
fn find_rows(corpus_path: &Path) -> std::io::Result<BTreeMap<&'static str, Vec<usize>>> {
    let reader = BufReader::new(File::open(corpus_path)?);
    let mut found: BTreeMap<&'static str, Vec<usize>> = BTreeMap::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let row: QuestionOnly = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let question = row.question.unwrap_or_default();
        if let Some(name) = group_of(&question) {
            found.entry(name).or_default().push(index);
        }
    }
    Ok(found)
}

/// This function prints the known-bad table.
pub fn print_table(corpus_path: &Path, scored: &[ScoredRow]) -> std::io::Result<()> {
    let by_row: BTreeMap<usize, &ScoredRow> = scored.iter().map(|row| (row.row_id, row)).collect();
    let found = find_rows(corpus_path)?;

    println!("=== THE FIVE KNOWN-BAD ROWS ===");
    println!();
    println!(
        "{:<32} {:>7} {:>8} {:>10} {:>10} {:>10}  penalised by ours?",
        "group", "in corpus", "scored", "ours mean", "ref mean", "err mean"
    );

    for group in groups() {
        let rows = found.get(group.name).cloned().unwrap_or_default();
        let scored_rows: Vec<&ScoredRow> = rows
            .iter()
            .filter_map(|id| by_row.get(id).copied())
            .collect();

        if scored_rows.is_empty() {
            println!(
                "{:<32} {:>7} {:>8} {:>10} {:>10} {:>10}  NOT IN THE SCORED SET",
                group.name,
                rows.len(),
                0,
                "-",
                "-",
                "-"
            );
            continue;
        }

        let ours: Vec<f64> = scored_rows.iter().map(|row| row.ours_bare).collect();
        let reference: Vec<f64> = scored_rows.iter().map(|row| row.ref_bare).collect();
        let errors: Vec<f64> = scored_rows
            .iter()
            .map(|row| (row.miner_c - row.actual_c).abs())
            .collect();

        // The corpus mean is the bar. A known-bad group that scores at
        // or above the corpus mean is NOT penalised.
        let corpus_mean = mean(&scored.iter().map(|row| row.ours_bare).collect::<Vec<f64>>());
        let group_mean = mean(&ours);
        let verdict = if group_mean < corpus_mean * 0.5 {
            "yes"
        } else if group_mean < corpus_mean {
            "weakly"
        } else {
            "NO, scores at or above the corpus mean"
        };

        println!(
            "{:<32} {:>7} {:>8} {:>10.6} {:>10.6} {:>10.3}  {}",
            group.name,
            rows.len(),
            scored_rows.len(),
            group_mean,
            mean(&reference),
            mean(&errors),
            verdict
        );
    }

    let corpus_mean = mean(&scored.iter().map(|row| row.ours_bare).collect::<Vec<f64>>());
    println!();
    println!(
        "corpus mean of our score, for comparison: {corpus_mean:.6} (n = {})",
        scored.len()
    );
    println!();
    println!("why each group is or is not visible to the evaluator:");
    for group in groups() {
        println!("  {:<32} {}", group.name, group.defect);
        println!("  {:<32} visible: {}", "", group.visible_to_evaluator);
    }
    Ok(())
}
