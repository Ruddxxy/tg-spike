//! This module calculates the score metrics.
//!
//! Every function in this module returns a finite number in the
//! range 0.0 to 1.0. A high score is good. A low score is bad. The
//! value 1.0 means a perfect answer. The value 0.0 means the worst
//! answer. Every function returns 0.0 for input it cannot use, so
//! bad input never scores better than a wrong but well formed
//! answer.
//!
//! Internally, the `brier` and `log_loss` functions first work out
//! a raw loss value, where 0.0 is a perfect answer and 1.0 is the
//! worst answer. The functions then convert that loss into a score
//! with `loss_to_score`, which flips the direction to `1.0 - loss`.
//! The loss direction and the score direction are opposite on
//! purpose. The loss math (squared error, log loss) is easiest to
//! write in loss terms, but the protocol needs the output in score
//! terms. See `crate` for the reason the protocol needs a high
//! score to mean a good answer.

use crate::error::ScoreError;
use crate::math::{kahan_sum, ln, sort_total_order};
use crate::parse::{self, GroundTruth, Response};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// The smallest and largest value the log loss clamp allows.
///
/// The code clamps `confidence` into `[EPS, 1 - EPS]` before it
/// calls `ln`. This stops `ln` from ever reading exactly 0.0, which
/// would return negative infinity.
pub const EPS: f64 = 1e-15;

/// The normalization constant for log loss.
///
/// This value is `-ln(EPS)`. The raw log loss value has no upper
/// bound, but every loss value in this crate must sit in the range
/// 0.0 to 1.0 before `loss_to_score` converts it. So `log_loss`
/// divides the raw loss by this constant. A unit test in the `math`
/// module checks that the hand written `ln` function agrees with
/// this literal. This constant stays in loss units. It never holds
/// a score value, so its name and its value still agree after the
/// score direction flip.
pub const MAX_LOSS: f64 = 34.538_776_394_910_684;

/// This function makes sure a loss value is a finite number in the
/// range 0.0 to 1.0.
///
/// The function returns 1.0, the worst possible loss, if the value
/// is NaN, positive infinity, or negative infinity. NaN must never
/// reach the output, because a NaN bit pattern is not the same on
/// every WASM host. This function works in loss units, where 1.0 is
/// bad. Callers that need a score in score units, where 0.0 is bad,
/// must run the result through `loss_to_score`.
fn clamp_loss(value: f64) -> f64 {
    if !value.is_finite() {
        return 1.0;
    }
    value.clamp(0.0, 1.0)
}

/// This function turns a loss value into a score.
///
/// The score is `1.0 - loss`. A loss of 0.0 is a perfect answer, and
/// it gives a score of 1.0, the best score. A loss of 1.0 is the
/// worst possible loss, and it gives a score of 0.0, the worst
/// score. The function clamps its result into `[0.0, 1.0]` again, so
/// a loss value outside `[0.0, 1.0]` can never push the score out of
/// range.
fn loss_to_score(loss: f64) -> f64 {
    (1.0 - loss).clamp(0.0, 1.0)
}

/// This function makes sure a score is a finite number in the range
/// 0.0 to 1.0.
///
/// The function returns 0.0, the worst score, if the value is NaN,
/// positive infinity, or negative infinity. This function works in
/// score units, where 0.0 is bad. It is not the same function as
/// `clamp_loss`, which works in loss units, where 1.0 is bad.
fn clamp_score(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

/// This function calculates the Brier score for one pair.
///
/// The function first works out the raw squared error,
/// `(confidence - label)^2`, which is a loss value: 0.0 for a
/// perfect answer, 1.0 for the worst answer. The function then
/// converts that loss into a score with `loss_to_score`. A perfect
/// answer, where `confidence` equals `label`, scores 1.0, the best
/// score. The worst answer scores 0.0, the worst score.
pub fn brier(label: u8, confidence: f64) -> f64 {
    let label_f = f64::from(label);
    let diff = confidence - label_f;
    let loss = clamp_loss(diff * diff);
    loss_to_score(loss)
}

/// This function calculates the log loss score for one pair.
///
/// The function first clamps `confidence` into `[EPS, 1 - EPS]`.
/// The function then calculates the raw log loss:
/// `-( label * ln(p) + (1 - label) * ln(1 - p) )`. The raw value has
/// no upper bound, so the function divides it by `MAX_LOSS` and
/// clamps the result into a loss in `[0.0, 1.0]`. This normalization
/// is a deliberate design choice, not an accident: it turns an
/// unbounded loss into a value that fits a fixed range, the same way
/// every loss in this crate does before conversion. A raw loss at or
/// above `MAX_LOSS` clamps to the worst loss, 1.0, which
/// `loss_to_score` then turns into the worst score, 0.0.
pub fn log_loss(label: u8, confidence: f64) -> f64 {
    let p = confidence.clamp(EPS, 1.0 - EPS);
    let label_f = f64::from(label);
    let raw = -(label_f * ln(p) + (1.0 - label_f) * ln(1.0 - p));
    let loss = clamp_loss(raw / MAX_LOSS);
    loss_to_score(loss)
}

/// This function builds a small table of named metric scores.
///
/// The function uses `BTreeMap`, not `HashMap`. A `BTreeMap` keeps
/// its keys in one fixed sorted order on every host. A `HashMap`
/// does not promise a fixed iteration order, which would break the
/// determinism this crate needs.
pub fn metrics_table(label: u8, confidence: f64) -> BTreeMap<&'static str, f64> {
    let mut table = BTreeMap::new();
    table.insert("brier", brier(label, confidence));
    table.insert("log_loss", log_loss(label, confidence));
    table
}

/// This function parses a ground truth pair and a response pair
/// from raw bytes. Then it calculates the Brier score.
///
/// The function returns an error if either input is not correct.
pub fn brier_from_bytes(gt_bytes: &[u8], resp_bytes: &[u8]) -> Result<f64, ScoreError> {
    let gt = parse::parse_ground_truth(gt_bytes)?;
    let resp = parse::parse_response(resp_bytes)?;
    Ok(brier(gt.label, resp.confidence))
}

/// This function parses a ground truth pair and a response pair
/// from raw bytes. Then it calculates the log loss score.
///
/// The function returns an error if either input is not correct.
pub fn log_loss_from_bytes(gt_bytes: &[u8], resp_bytes: &[u8]) -> Result<f64, ScoreError> {
    let gt = parse::parse_ground_truth(gt_bytes)?;
    let resp = parse::parse_response(resp_bytes)?;
    Ok(log_loss(gt.label, resp.confidence))
}

/// This struct holds one row of a `score_batch` input array, before
/// the code validates the `ground_truth` and `response` fields.
///
/// The struct uses `#[derive(Deserialize)]` so `serde_json` can
/// build it straight from a JSON array element. `serde_json` skips
/// any field this struct does not name, so unknown extra keys on a
/// batch row are ignored.
#[derive(Deserialize)]
struct BatchRow {
    /// The raw ground truth value for this row. See `parse` for the
    /// field-level validation this crate runs on it.
    ground_truth: Value,
    /// The raw response value for this row. See `parse` for the
    /// field-level validation this crate runs on it.
    response: Value,
}

/// This function calculates the mean Brier score for a batch of
/// pairs.
///
/// The parameter `bytes` holds a JSON array. Each array element is
/// an object with a `ground_truth` field and a `response` field,
/// for example `{"ground_truth": {"label": 1}, "response":
/// {"confidence": 0.75}}`. The function returns an error if `bytes`
/// is not valid UTF-8 text, not valid JSON, or not a JSON array. The
/// `score_batch` export in the `abi` module turns that error into
/// the worst score, 0.0, the same way it turns every other error in
/// this crate into 0.0. The function returns `Ok(0.0)` for a well
/// formed but empty array. An empty array holds no pair to score, so
/// the function treats it as the worst case the same way a real
/// scoring failure would be treated.
///
/// A single bad element does not fail the whole batch. If one
/// element is missing a field, has the wrong type, or holds an
/// out-of-range value, that element scores 0.0 on its own, and the
/// function still scores every other element normally. This keeps
/// one bad row from hiding the score of every good row next to it.
///
/// The function sorts the per-element scores into a fixed total
/// order and adds them with Kahan summation before it divides by
/// the count. See the `math` module doc comment for the reason.
/// Together the sort and the Kahan method make sure the result does
/// not depend on the order the caller gives the pairs in.
///
/// ## Why this function converts each element before it averages
///
/// This function scores each element with `score_one_batch_row`
/// first, so every value in `scores` is already a converted score
/// (`1.0 - loss`), and only then takes the mean of those scores. It
/// does not take the mean of the raw per-element losses and convert
/// the mean once at the end. In exact real-number math the two
/// paths give the same answer, because `mean(1 - L_i)` equals
/// `1 - mean(L_i)`. This code does not rely on that equality, for
/// three reasons.
///
/// First, floating point addition is not associative, and this
/// function does more than plain addition: it sorts the values into
/// a fixed total order and adds them with Kahan summation. The
/// sorted order of the converted scores `{1 - L_i}` is the reverse
/// of the sorted order of the raw losses `{L_i}`. A reversed order
/// changes which values add together first, and Kahan summation
/// carries a running compensation term that depends on that order.
/// The two paths are not bit-identical, even though they are equal
/// in exact math. This crate treats bit-determinism as a
/// correctness requirement, not a nicety, so the two paths are not
/// interchangeable here.
///
/// Second, each element clamps into `[0.0, 1.0]` on its own, inside
/// `brier` and `log_loss`, before this function ever sees the
/// value. Clamping each element and then taking the mean is not the
/// same operation as taking the mean first and clamping the mean
/// once, whenever an element would have fallen outside the range
/// before its own clamp ran.
///
/// Third, the protocol scores one miner response at a time. A score
/// worked out per element, right where that element is scored, is
/// the model that matches the protocol. Converting per element,
/// then averaging, keeps that model in the batch path too.
pub fn batch_brier_from_bytes(bytes: &[u8]) -> Result<f64, ScoreError> {
    let text = core::str::from_utf8(bytes).map_err(|_| ScoreError::InvalidUtf8)?;
    let elements: Vec<Value> = serde_json::from_str(text).map_err(|_| ScoreError::InvalidJson)?;
    if elements.is_empty() {
        return Ok(0.0);
    }

    let mut scores: Vec<f64> = Vec::with_capacity(elements.len());
    for element in &elements {
        scores.push(score_one_batch_row(element));
    }

    sort_total_order(&mut scores);
    let sum = kahan_sum(&scores);
    let count = scores.len() as f64;
    Ok(clamp_score(sum / count))
}

/// This function scores one row of a batch array.
///
/// The function returns 0.0 if the row is not an object with a
/// `ground_truth` field and a `response` field, or if either field
/// does not pass the checks in the `parse` module.
fn score_one_batch_row(element: &Value) -> f64 {
    parse_and_score_row(element).unwrap_or(0.0)
}

/// This function does the fallible work behind `score_one_batch_row`.
fn parse_and_score_row(element: &Value) -> Result<f64, ScoreError> {
    let row: BatchRow =
        serde_json::from_value(element.clone()).map_err(|_| ScoreError::WrongType("batch_row"))?;
    let gt: GroundTruth = parse::ground_truth_from_value(&row.ground_truth)?;
    let resp: Response = parse::response_from_value(&row.response)?;
    Ok(brier(gt.label, resp.confidence))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brier_matches_golden_values() {
        assert_eq!(brier(0, 0.0), 1.0);
        assert_eq!(brier(1, 1.0), 1.0);
        assert_eq!(brier(0, 1.0), 0.0);
        assert_eq!(brier(1, 0.0), 0.0);
        assert_eq!(brier(0, 0.5), 0.75);
        assert_eq!(brier(1, 0.5), 0.75);
        assert_eq!(brier(1, 0.75), 0.9375);
        assert_eq!(brier(0, 0.25), 0.9375);
        assert_eq!(brier(1, 0.125), 0.234375);
        assert_eq!(brier(0, 0.875), 0.234375);
    }

    #[test]
    fn log_loss_is_near_one_for_a_near_perfect_answer() {
        let score = log_loss(1, 1.0);
        assert!(
            score > 1.0 - 1e-9,
            "log_loss(1, 1.0) = {score}, want near 1.0"
        );
        let score = log_loss(0, 0.0);
        assert!(
            score > 1.0 - 1e-9,
            "log_loss(0, 0.0) = {score}, want near 1.0"
        );
    }

    #[test]
    fn log_loss_is_worst_for_a_confident_wrong_answer() {
        let score = log_loss(0, 1.0);
        assert!(score < 1e-9, "log_loss(0, 1.0) = {score}, want near 0.0");
        let score = log_loss(1, 0.0);
        assert!(score < 1e-9, "log_loss(1, 0.0) = {score}, want near 0.0");
    }

    #[test]
    fn log_loss_no_information_answer_is_mid_range() {
        let score = log_loss(1, 0.5);
        assert!((0.0..1.0).contains(&score));
    }

    #[test]
    fn scores_never_leave_the_unit_range() {
        for label in [0u8, 1u8] {
            for step in 0..=20 {
                let confidence = f64::from(step) / 20.0;
                let brier_score = brier(label, confidence);
                let loss_score = log_loss(label, confidence);
                assert!((0.0..=1.0).contains(&brier_score));
                assert!((0.0..=1.0).contains(&loss_score));
                assert!(brier_score.is_finite());
                assert!(loss_score.is_finite());
            }
        }
    }

    #[test]
    fn brier_from_bytes_matches_golden_vector() {
        let gt = b"{\"label\": 1}";
        let resp = b"{\"confidence\": 0.75}";
        assert_eq!(brier_from_bytes(gt, resp).unwrap(), 0.9375);
    }

    #[test]
    fn brier_from_bytes_reports_bad_input() {
        assert!(brier_from_bytes(b"not json", b"{\"confidence\": 0.5}").is_err());
        assert!(brier_from_bytes(b"{\"label\": 1}", b"not json").is_err());
    }

    #[test]
    fn batch_of_empty_array_is_worst_score() {
        assert_eq!(batch_brier_from_bytes(b"[]").unwrap(), 0.0);
    }

    #[test]
    fn batch_of_malformed_top_level_input_is_an_error() {
        // A malformed top level input is an `Err` from this pure
        // function. The `score_batch` export in `abi` is the layer
        // that turns any `Err` into the worst score, 0.0.
        assert!(batch_brier_from_bytes(b"not json").is_err());
        assert!(batch_brier_from_bytes(b"{}").is_err());
        assert!(batch_brier_from_bytes(&[0xff, 0xfe]).is_err());
    }

    #[test]
    fn batch_averages_two_perfect_pairs_to_one() {
        let input = br#"[
            {"ground_truth": {"label": 1}, "response": {"confidence": 1.0}},
            {"ground_truth": {"label": 0}, "response": {"confidence": 0.0}}
        ]"#;
        assert_eq!(batch_brier_from_bytes(input).unwrap(), 1.0);
    }

    #[test]
    fn batch_isolates_one_bad_row() {
        let input = br#"[
            {"ground_truth": {"label": 1}, "response": {"confidence": 1.0}},
            {"ground_truth": {"label": 0}}
        ]"#;
        // The second row is missing "response". It scores 0.0 on
        // its own. The first row is a perfect answer and scores
        // 1.0. The mean of 1.0 and 0.0 is 0.5.
        assert_eq!(batch_brier_from_bytes(input).unwrap(), 0.5);
    }

    #[test]
    fn batch_score_equals_mean_of_converted_per_item_scores() {
        // This test builds a batch by hand and checks that
        // `batch_brier_from_bytes` matches the mean of the already
        // converted per-item `brier` scores, not the converted mean
        // of the raw per-item losses. See the doc comment on
        // `batch_brier_from_bytes` for why the two are not always
        // bit-identical in general, even though they agree here
        // within the loose tolerance this test uses.
        let pairs = [(1u8, 0.9), (0u8, 0.1), (1u8, 0.4), (0u8, 0.6), (1u8, 0.55)];
        let mut body = String::from("[");
        for (index, (label, confidence)) in pairs.iter().enumerate() {
            if index > 0 {
                body.push(',');
            }
            body.push_str(&format!(
                "{{\"ground_truth\": {{\"label\": {label}}}, \"response\": {{\"confidence\": {confidence}}}}}"
            ));
        }
        body.push(']');

        let expected_mean: f64 = pairs
            .iter()
            .map(|(label, confidence)| brier(*label, *confidence))
            .sum::<f64>()
            / pairs.len() as f64;

        let actual = batch_brier_from_bytes(body.as_bytes()).unwrap();
        assert!(
            (actual - expected_mean).abs() < 1e-12,
            "batch score {actual} did not match hand computed mean {expected_mean}"
        );
    }

    #[test]
    fn metrics_table_has_both_named_metrics() {
        let table = metrics_table(1, 0.75);
        assert_eq!(table.len(), 2);
        assert_eq!(table.get("brier"), Some(&0.9375));
        assert!(table.contains_key("log_loss"));
        // BTreeMap keeps keys in sorted order. "brier" sorts before
        // "log_loss".
        let keys: Vec<&&str> = table.keys().collect();
        assert_eq!(keys, vec![&"brier", &"log_loss"]);
    }
}
