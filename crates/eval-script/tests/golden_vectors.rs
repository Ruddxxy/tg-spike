//! This test pins the golden vectors for the scoring module.
//!
//! Each vector names a question, a ground truth, a miner answer, and
//! the exact `f32` bit pattern the module must return. The host
//! runner reads the same vectors and checks them through wasmtime and
//! through wazero, so a change here is visible on every host.
//!
//! The old vectors came from the Brier model, which scored a JSON
//! label against a JSON confidence. That model is gone. Every vector
//! below comes from the new scorer.
//!
//! The bit pattern is the unit of comparison, not the decimal text. A
//! validator whose score differs by one bit from the network median
//! is at risk of a slash, so the test compares bits.

use eval_script::score::score_answer;

/// One golden vector.
struct Vector {
    /// The name that the report and the host runner use.
    name: &'static str,
    /// The question text. It may be junk, and it may be empty.
    question: &'static str,
    /// The ground truth text.
    ground_truth: &'static str,
    /// The miner answer text.
    miner_answer: &'static str,
    /// The exact `f32` bit pattern the module must return.
    bits: u32,
}

/// The golden vectors.
///
/// These values match `golden_vectors.json` at the workspace root.
/// The file and this table must stay in step; the test at the end of
/// this file checks that they do.
const VECTORS: [Vector; 16] = [
    Vector {
        name: "numeric_exact",
        question: "",
        ground_truth: "192.43",
        miner_answer: "192.43",
        bits: 0x3f80_0000,
    },
    Vector {
        name: "numeric_one_cent_out",
        question: "",
        ground_truth: "192.43",
        miner_answer: "192.44",
        bits: 0x3f7f_ffce,
    },
    Vector {
        name: "numeric_trailing_zero",
        question: "",
        ground_truth: "192.43",
        miner_answer: "192.430",
        bits: 0x3f80_0000,
    },
    Vector {
        name: "numeric_wild_answer",
        question: "",
        ground_truth: "192.43",
        miner_answer: "999999.99",
        bits: 0x2e12_a09c,
    },
    Vector {
        name: "numeric_one_percent_out",
        question: "",
        ground_truth: "100",
        miner_answer: "101",
        bits: 0x3f66_6666,
    },
    Vector {
        name: "numeric_fifty_percent_out",
        question: "",
        ground_truth: "100",
        miner_answer: "150",
        bits: 0x3b6b_1553,
    },
    Vector {
        name: "currency_symbol_match",
        question: "",
        ground_truth: "192.43",
        miner_answer: "$192.43",
        bits: 0x3f80_0000,
    },
    Vector {
        name: "currency_code_match",
        question: "",
        ground_truth: "192.43",
        miner_answer: "192.43 USD",
        bits: 0x3f80_0000,
    },
    Vector {
        name: "unit_kelvin_to_celsius",
        question: "",
        ground_truth: "34.7 C",
        miner_answer: "307.85 K",
        bits: 0x3f80_0000,
    },
    Vector {
        name: "unit_incompatible",
        question: "",
        ground_truth: "307.85 K",
        miner_answer: "307.85 USD",
        bits: 0x0000_0000,
    },
    Vector {
        name: "text_exact",
        question: "",
        ground_truth: "malicious",
        miner_answer: "malicious",
        bits: 0x3f80_0000,
    },
    Vector {
        name: "text_unrelated",
        question: "",
        ground_truth: "malicious",
        miner_answer: "sunny",
        bits: 0x0000_0000,
    },
    Vector {
        name: "text_negated",
        question: "",
        ground_truth: "malicious",
        miner_answer: "not malicious",
        bits: 0x0000_0000,
    },
    Vector {
        name: "text_is_attack",
        question: "",
        ground_truth: "is malicious",
        miner_answer: "is",
        bits: 0x3f00_0000,
    },
    Vector {
        name: "blank_answer",
        question: "",
        ground_truth: "192.43",
        miner_answer: "",
        bits: 0x0000_0000,
    },
    Vector {
        name: "question_is_junk",
        question: "[direct] 207 -> /price",
        ground_truth: "192.43",
        miner_answer: "192.43",
        bits: 0x3f80_0000,
    },
];

#[test]
fn every_golden_vector_matches_its_bit_pattern() {
    for vector in VECTORS.iter() {
        let value = score_answer(vector.question, vector.ground_truth, vector.miner_answer) as f32;
        assert_eq!(
            value.to_bits(),
            vector.bits,
            "vector {}: got {} (0x{:08x}), want 0x{:08x}",
            vector.name,
            value,
            value.to_bits(),
            vector.bits
        );
    }
}

#[test]
fn every_golden_vector_sits_inside_the_closed_range() {
    for vector in VECTORS.iter() {
        let value = score_answer(vector.question, vector.ground_truth, vector.miner_answer) as f32;
        assert!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "vector {} gave {value}, which is outside [0, 1]",
            vector.name
        );
    }
}

#[test]
fn the_vector_names_are_unique() {
    // A repeated name would make a host runner report ambiguous.
    for (index, vector) in VECTORS.iter().enumerate() {
        for other in VECTORS.iter().skip(index + 1) {
            assert_ne!(
                vector.name, other.name,
                "the name {} appears twice",
                vector.name
            );
        }
    }
}

#[test]
fn the_json_file_matches_this_table() {
    // The host runner and the wazero runner both read the JSON file at
    // the workspace root. This test reads that same file, not a copy,
    // because a copy could drift and let the two hosts check different
    // things while this test still passed.
    let text = include_str!("../../../golden_vectors.json");
    for vector in VECTORS.iter() {
        let name_key = alloc_name_key(vector.name);
        assert!(
            text.contains(&name_key),
            "the JSON file has no vector named {}",
            vector.name
        );
        let bits_text = alloc_bits_text(vector.bits);
        assert!(
            text.contains(&bits_text),
            "the JSON file has no bit pattern {} for vector {}",
            bits_text,
            vector.name
        );
    }
}

/// This function builds the JSON key text for a vector name.
fn alloc_name_key(name: &str) -> String {
    let mut key = String::from("\"name\": \"");
    key.push_str(name);
    key.push('"');
    key
}

/// This function builds the JSON bit pattern text for a vector.
fn alloc_bits_text(bits: u32) -> String {
    format!("\"0x{bits:08x}\"")
}
