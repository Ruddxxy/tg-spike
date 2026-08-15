//! This module reads the corpus and prepares it for scoring.
//!
//! ## Why a preparation step exists
//!
//! The corpus stores the FULL upstream miner response for each row,
//! because it is built as a record of what the daemon feed sent.
//! The protocol does not give `rank_answer` that blob. The protocol
//! team states that a miner answer is "a single extracted value from
//! the miner's signal_mapping, standardized internally before it
//! reaches rank_answer".
//!
//! So this step does the extraction the protocol does internally. It
//! uses `corpus_builder::schema::normalise`, which is the SAME code
//! that built the corpus. A second copy of that logic would drift from
//! the first, and then the evaluation would measure the copy.
//!
//! The step writes a small file that the wazero runner reads. Keeping
//! the extraction in Rust and the scoring in Go means the extraction
//! exists once, not twice.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use corpus_builder::schema;

/// One row as the corpus stores it.
#[derive(Debug, Deserialize)]
pub struct CorpusRow {
    /// The question text the daemon feed carried.
    pub question: Option<String>,
    /// The bare ground truth, for example "28.9".
    pub gt_bare: Option<String>,
    /// The prose ground truth.
    pub gt_prose: Option<String>,
    /// The JSON ground truth.
    pub gt_json: Option<String>,
    /// The FULL upstream miner response, as a JSON text.
    pub miner_answer: Option<String>,
    /// The miner slug.
    pub miner_slug: String,
    /// The intent name, when the feed gave one.
    pub intent: Option<String>,
    /// The valid time this row scores, in the archive time format.
    pub valid_time: String,
    /// The archive-derived actual temperature, in Celsius.
    pub actual_c: Option<f64>,
    /// The paraphrase cluster, when the row has one.
    pub cluster_id: Option<String>,
}

/// One row as the scoring step reads it.
#[derive(Debug, Serialize, Deserialize)]
pub struct EvalRow {
    /// A stable row number, so a report can name one row.
    pub row_id: usize,
    /// The question text. It may be empty.
    pub question: String,
    /// The bare ground truth rendering.
    pub gt_bare: String,
    /// The prose ground truth rendering.
    pub gt_prose: String,
    /// The JSON ground truth rendering.
    pub gt_json: String,
    /// The SINGLE extracted miner value, as the protocol would give it.
    pub miner_value: String,
    /// The miner slug.
    pub miner_slug: String,
    /// The intent name, or an empty text.
    pub intent: String,
    /// The valid time.
    pub valid_time: String,
    /// The archive actual, in Celsius.
    pub actual_c: f64,
    /// The miner's own claimed value, in Celsius, as a number.
    pub miner_c: f64,
    /// The cluster, or an empty text.
    pub cluster_id: String,
}

/// What the preparation step found.
pub struct PrepareReport {
    /// Rows read from the corpus.
    pub rows_read: usize,
    /// Rows with all three ground-truth renderings and an actual.
    pub rows_with_truth: usize,
    /// Rows where the extraction produced a value for the valid time.
    pub rows_written: usize,
    /// Why a row was dropped, counted by reason.
    pub drop_reasons: BTreeMap<String, usize>,
}

/// This function reads the corpus and writes the scoring input.
///
/// The function returns a report of what it kept and what it dropped.
pub fn prepare(input: &Path, output: &Path) -> std::io::Result<PrepareReport> {
    let reader = BufReader::new(File::open(input)?);
    let mut writer = BufWriter::new(File::create(output)?);

    let mut report = PrepareReport {
        rows_read: 0,
        rows_with_truth: 0,
        rows_written: 0,
        drop_reasons: BTreeMap::new(),
    };

    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        report.rows_read += 1;

        let row: CorpusRow = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                bump(&mut report.drop_reasons, "corpus_row_unreadable");
                continue;
            }
        };

        let (Some(gt_bare), Some(gt_prose), Some(gt_json), Some(actual_c)) = (
            row.gt_bare.clone(),
            row.gt_prose.clone(),
            row.gt_json.clone(),
            row.actual_c,
        ) else {
            bump(&mut report.drop_reasons, "no_ground_truth");
            continue;
        };
        report.rows_with_truth += 1;

        let Some(answer_text) = row.miner_answer.clone() else {
            bump(&mut report.drop_reasons, "no_miner_answer");
            continue;
        };
        let answer_value: Value = match serde_json::from_str(&answer_text) {
            Ok(value) => value,
            Err(_) => {
                bump(&mut report.drop_reasons, "miner_answer_not_json");
                continue;
            }
        };

        // Run the SAME normaliser the corpus was built with.
        let points = match schema::normalise(&row.miner_slug, &answer_value) {
            Ok(points) => points,
            Err(reason) => {
                bump(&mut report.drop_reasons, reason.label());
                continue;
            }
        };

        // Keep the point for this row's valid time. The corpus emits one
        // row per (response, valid time) pair, so exactly one point
        // should match.
        let Some(point) = points
            .iter()
            .find(|point| point.valid_time_utc == row.valid_time)
        else {
            bump(&mut report.drop_reasons, "no_point_at_valid_time");
            continue;
        };

        let eval_row = EvalRow {
            row_id: index,
            question: row.question.unwrap_or_default(),
            gt_bare,
            gt_prose,
            gt_json,
            // The extracted value, rendered as a bare Celsius number.
            // This is the shape the protocol standardises to: one
            // value, no wrapper.
            miner_value: format_value(point.temp_c),
            miner_slug: row.miner_slug,
            intent: row.intent.unwrap_or_default(),
            valid_time: row.valid_time,
            actual_c,
            miner_c: point.temp_c,
            cluster_id: row.cluster_id.unwrap_or_default(),
        };
        let encoded = serde_json::to_string(&eval_row)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        writeln!(writer, "{encoded}")?;
        report.rows_written += 1;
    }

    writer.flush()?;
    Ok(report)
}

/// This function renders a Celsius value as a miner would send it.
///
/// The function keeps one decimal place, which is the precision the
/// ground-truth renderings use. A miner that reports more precision
/// than the truth carries gains nothing from it, and the extra digits
/// would only make the two texts differ for no reason.
fn format_value(celsius: f64) -> String {
    format!("{celsius:.1}")
}

/// This function adds one to a count in a map.
fn bump(counts: &mut BTreeMap<String, usize>, key: &str) {
    *counts.entry(key.to_string()).or_insert(0) += 1;
}

/// This function reads the prepared rows back.
pub fn load_eval_rows(path: &Path) -> std::io::Result<Vec<EvalRow>> {
    let reader = BufReader::new(File::open(path)?);
    let mut rows = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(row) => rows.push(row),
            Err(error) => {
                return Err(std::io::Error::other(format!(
                    "cannot read a prepared row: {error}"
                )))
            }
        }
    }
    Ok(rows)
}

/// This function reads the RAW upstream miner responses.
///
/// The result is the full JSON blob for each corpus row. The scoring
/// module never receives this text; the protocol extracts a single
/// value first. The coverage report uses it only to show what the
/// parser does with text far outside its shape.
pub fn raw_miner_answers(path: &Path) -> std::io::Result<Vec<String>> {
    let reader = BufReader::new(File::open(path)?);
    let mut answers = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let row: CorpusRow = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(answer) = row.miner_answer {
            answers.push(answer);
        }
    }
    Ok(answers)
}
