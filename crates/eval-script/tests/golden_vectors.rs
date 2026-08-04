//! This test drives the pure Rust `eval_script::metrics` API against
//! the golden vectors for the Track 2 evaluation script.
//!
//! The workspace root holds `golden_vectors.json`. This crate must
//! not touch that file, because a different agent owns it. This
//! test hardcodes the same vectors here instead, as the crate spec
//! allows. Each `expected` value is a dyadic rational, so an `f64`
//! holds it exactly, and this test compares the exact bit pattern
//! with `assert_eq!`, not a tolerance.
//!
//! A high score is good, so `expected` here holds the score, not the
//! raw Brier loss. Each value equals `1.0 - old_brier_loss`. See
//! `golden_vectors.json` at the workspace root for the same numbers
//! with the same names.

struct Vector {
    name: &'static str,
    ground_truth: &'static str,
    response: &'static str,
    expected: f64,
}

const VECTORS: &[Vector] = &[
    Vector {
        name: "perfect_negative",
        ground_truth: "{\"label\": 0}",
        response: "{\"confidence\": 0.0}",
        expected: 1.0,
    },
    Vector {
        name: "perfect_positive",
        ground_truth: "{\"label\": 1}",
        response: "{\"confidence\": 1.0}",
        expected: 1.0,
    },
    Vector {
        name: "worst_negative",
        ground_truth: "{\"label\": 0}",
        response: "{\"confidence\": 1.0}",
        expected: 0.0,
    },
    Vector {
        name: "worst_positive",
        ground_truth: "{\"label\": 1}",
        response: "{\"confidence\": 0.0}",
        expected: 0.0,
    },
    Vector {
        name: "no_information_negative",
        ground_truth: "{\"label\": 0}",
        response: "{\"confidence\": 0.5}",
        expected: 0.75,
    },
    Vector {
        name: "no_information_positive",
        ground_truth: "{\"label\": 1}",
        response: "{\"confidence\": 0.5}",
        expected: 0.75,
    },
    Vector {
        name: "good_positive_quarter",
        ground_truth: "{\"label\": 1}",
        response: "{\"confidence\": 0.75}",
        expected: 0.9375,
    },
    Vector {
        name: "good_negative_quarter",
        ground_truth: "{\"label\": 0}",
        response: "{\"confidence\": 0.25}",
        expected: 0.9375,
    },
    Vector {
        name: "bad_positive_eighth",
        ground_truth: "{\"label\": 1}",
        response: "{\"confidence\": 0.125}",
        expected: 0.234375,
    },
    Vector {
        name: "bad_negative_eighth",
        ground_truth: "{\"label\": 0}",
        response: "{\"confidence\": 0.875}",
        expected: 0.234375,
    },
    Vector {
        name: "integer_confidence_zero",
        ground_truth: "{\"label\": 1}",
        response: "{\"confidence\": 0}",
        expected: 0.0,
    },
    Vector {
        name: "integer_confidence_one",
        ground_truth: "{\"label\": 0}",
        response: "{\"confidence\": 1}",
        expected: 0.0,
    },
    Vector {
        name: "extra_fields_are_ignored",
        ground_truth: "{\"label\": 1, \"source\": \"oracle\"}",
        response: "{\"confidence\": 0.75, \"model\": \"m1\"}",
        expected: 0.9375,
    },
    Vector {
        name: "whitespace_is_ignored",
        ground_truth: "  {  \"label\" : 0 }  ",
        response: "\n{\n  \"confidence\"\t: 0.25\n}\n",
        expected: 0.9375,
    },
];

#[test]
fn every_golden_vector_matches_the_exact_expected_bits() {
    for vector in VECTORS {
        let actual = eval_script::metrics::brier_from_bytes(
            vector.ground_truth.as_bytes(),
            vector.response.as_bytes(),
        )
        .unwrap_or_else(|err| panic!("vector \"{}\" failed to parse: {err}", vector.name));
        assert_eq!(
            actual.to_bits(),
            vector.expected.to_bits(),
            "vector \"{}\" gave {actual}, want {}",
            vector.name,
            vector.expected
        );
    }
}

#[test]
fn every_golden_vector_matches_through_the_batch_path() {
    // Run every golden vector through `score_batch` as one array, in
    // its declared order, and check the mean by hand with the
    // hardcoded expected values. This exercises the batch path
    // against the same trusted numbers as the single-pair path.
    let mut body = String::from("[");
    for (index, vector) in VECTORS.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&format!(
            "{{\"ground_truth\": {}, \"response\": {}}}",
            vector.ground_truth, vector.response
        ));
    }
    body.push(']');

    let expected_mean: f64 =
        VECTORS.iter().map(|vector| vector.expected).sum::<f64>() / VECTORS.len() as f64;

    let actual = eval_script::metrics::batch_brier_from_bytes(body.as_bytes())
        .expect("the combined golden vector batch must parse");
    assert!(
        (actual - expected_mean).abs() < 1e-12,
        "batch mean {actual} did not match hand computed mean {expected_mean}"
    );
}
