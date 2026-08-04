//! This test checks that `score_batch` gives the exact same bit
//! pattern back for the same multiset of pairs, no matter what order
//! the host gives the pairs in.
//!
//! Float addition is not associative, so a plain running sum over
//! the pairs in array order would not have this property. The
//! `metrics` module sorts the per-pair scores into a fixed total
//! order and adds them with Kahan summation before it takes the
//! mean, specifically so this property holds. See the `math` module
//! doc comment for the full reason.

use eval_script::metrics::batch_brier_from_bytes;

/// This struct holds one row for the batch JSON array this test
/// builds.
struct Row {
    label: u8,
    confidence: f64,
}

fn row_json(row: &Row) -> String {
    format!(
        "{{\"ground_truth\": {{\"label\": {}}}, \"response\": {{\"confidence\": {}}}}}",
        row.label, row.confidence
    )
}

fn batch_json(order: &[usize], rows: &[Row]) -> String {
    let mut body = String::from("[");
    for (position, &row_index) in order.iter().enumerate() {
        if position > 0 {
            body.push(',');
        }
        body.push_str(&row_json(&rows[row_index]));
    }
    body.push(']');
    body
}

#[test]
fn batch_mean_is_bit_identical_across_many_orderings() {
    let rows = [
        Row {
            label: 1,
            confidence: 0.9,
        },
        Row {
            label: 0,
            confidence: 0.1,
        },
        Row {
            label: 1,
            confidence: 0.3,
        },
        Row {
            label: 0,
            confidence: 0.7,
        },
        Row {
            label: 1,
            confidence: 0.55,
        },
        Row {
            label: 0,
            confidence: 0.05,
        },
        Row {
            label: 1,
            confidence: 0.999,
        },
        Row {
            label: 0,
            confidence: 0.001,
        },
    ];

    let orderings: &[&[usize]] = &[
        &[0, 1, 2, 3, 4, 5, 6, 7],
        &[7, 6, 5, 4, 3, 2, 1, 0],
        &[3, 1, 4, 6, 5, 2, 0, 7],
        &[0, 2, 4, 6, 1, 3, 5, 7],
        &[7, 0, 6, 1, 5, 2, 4, 3],
        &[2, 5, 1, 7, 0, 4, 6, 3],
    ];

    let baseline_json = batch_json(orderings[0], &rows);
    let baseline =
        batch_brier_from_bytes(baseline_json.as_bytes()).expect("a well formed batch must parse");

    for order in orderings {
        let json = batch_json(order, &rows);
        let result =
            batch_brier_from_bytes(json.as_bytes()).expect("a well formed batch must parse");
        assert_eq!(
            result.to_bits(),
            baseline.to_bits(),
            "order {order:?} gave {result}, baseline was {baseline}"
        );
    }
}

#[test]
fn batch_mean_is_bit_identical_with_a_bad_row_mixed_in() {
    // A malformed row scores 0.0 on its own and does not change the
    // count of rows that get averaged. Order independence must hold
    // just as well once a bad row is mixed into the good rows.
    let good_rows = [
        "{\"ground_truth\": {\"label\": 1}, \"response\": {\"confidence\": 0.8}}",
        "{\"ground_truth\": {\"label\": 0}, \"response\": {\"confidence\": 0.2}}",
        "{\"ground_truth\": {\"label\": 1}, \"response\": {\"confidence\": 0.6}}",
    ];
    let bad_row = "{\"ground_truth\": {\"label\": 1}}";

    let forward = format!(
        "[{},{},{},{}]",
        good_rows[0], good_rows[1], bad_row, good_rows[2]
    );
    let backward = format!(
        "[{},{},{},{}]",
        good_rows[2], bad_row, good_rows[1], good_rows[0]
    );

    let forward_score =
        batch_brier_from_bytes(forward.as_bytes()).expect("a well formed batch must parse");
    let backward_score =
        batch_brier_from_bytes(backward.as_bytes()).expect("a well formed batch must parse");

    assert_eq!(forward_score.to_bits(), backward_score.to_bits());
}
