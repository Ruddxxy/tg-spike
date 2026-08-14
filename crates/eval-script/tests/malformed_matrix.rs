//! This test drives the ABI safety contract with every malformed
//! input shape.
//!
//! The contract changed with the new scoring model. The old model
//! parsed both sides as JSON, so a malformed input was an ERROR that
//! the ABI turned into 0.0. Both sides are now free text, so almost
//! no text is malformed: a text that holds no value is simply a text
//! that scores badly. The rules that remain are these:
//!
//! - A pointer and length pair that names memory outside the module
//!   scores 0.0, and never reads that memory.
//! - A length over `MAX_INPUT_BYTES` scores 0.0, and the check runs
//!   before any bounds check and before any read.
//! - A blank miner answer scores exactly 0.0.
//! - A miner answer that is not UTF-8 scores 0.0.
//! - Every input gives a finite score inside `[0.0, 1.0]`.
//!
//! The calls below use `rank_answer_impl`, which is the one scoring
//! entry point. Each call passes a length of 0 or an already invalid
//! pointer, so no call ever reads a real memory address. See the
//! `abi` module doc comment for why a native target cannot safely
//! round-trip a real pointer through this ABI.

use eval_script::abi;
use eval_script::score::score_answer;

/// This helper calls the scoring export with a question of length 0.
fn rank(gt_ptr: u32, gt_len: u32, ma_ptr: u32, ma_len: u32) -> f32 {
    abi::rank_answer_impl(0, 0, gt_ptr, gt_len, ma_ptr, ma_len)
}

#[test]
fn a_pointer_and_length_pair_outside_memory_scores_the_worst_score() {
    // ptr + len overflows a u32. The ABI catches this before it reads
    // any memory and returns the worst score, 0.0.
    assert_eq!(rank(u32::MAX, u32::MAX, 0, 0), 0.0);
    assert_eq!(rank(0, 0, u32::MAX, u32::MAX), 0.0);
}

#[test]
fn an_integer_overflow_in_ptr_plus_len_scores_the_worst_score() {
    assert_eq!(rank(u32::MAX - 5, 10, 0, 0), 0.0);
    assert_eq!(rank(0, 0, u32::MAX - 5, 10), 0.0);
}

#[test]
fn a_null_pointer_with_a_nonzero_length_scores_the_worst_score() {
    // ptr is 0 (null) but len is nonzero, so the offset does not name
    // a real block. The ABI returns the worst score before it ever
    // dereferences the pointer.
    assert_eq!(rank(0, 8, 0, 0), 0.0);
    assert_eq!(rank(0, 0, 0, 8), 0.0);
}

#[test]
fn an_oversize_length_scores_the_worst_score_without_a_trap() {
    // The length is one byte over `MAX_INPUT_BYTES`. The pointer is a
    // wild address that is never a valid block. The ABI checks the
    // length cap before it checks bounds or reads memory, so this
    // never touches the wild pointer.
    let over_cap = eval_script::MAX_INPUT_BYTES + 1;
    assert_eq!(rank(0xdead_beef, over_cap, 0, 0), 0.0);
    assert_eq!(rank(0, 0, 0xdead_beef, over_cap), 0.0);
}

#[test]
fn a_zero_length_pair_scores_the_worst_score() {
    // Both sides are empty. The miner answer is blank, so the score
    // is exactly 0.0.
    assert_eq!(rank(0, 0, 0, 0), 0.0);
}

#[test]
fn a_junk_question_pointer_does_not_change_the_score() {
    // The question is advisory. A wild question pointer and an
    // oversize question length must both fall back to an empty
    // question, not to a failed score.
    let over_cap = eval_script::MAX_INPUT_BYTES + 1;
    let with_wild_question = abi::rank_answer_impl(0xdead_beef, over_cap, 0, 0, 0, 0);
    let with_no_question = abi::rank_answer_impl(0, 0, 0, 0, 0, 0);
    assert_eq!(with_wild_question, with_no_question);
}

// ---------------------------------------------------------------
// The text level contract, through the pure scoring function
// ---------------------------------------------------------------

#[test]
fn every_malformed_text_gives_a_score_inside_the_range() {
    // None of these is an error any more. Each one must give a
    // finite score inside the closed range.
    let cases: [(&str, &str); 14] = [
        ("", ""),
        ("", "192.43"),
        ("192.43", ""),
        ("{not json}", "{not json}"),
        ("[1, 2]", "{\"label\": \"1\"}"),
        ("null", "null"),
        ("N/A", "N/A"),
        ("\u{0}\u{1}", "\u{0}\u{1}"),
        ("1e400", "1e400"),
        ("-1e400", "192.43"),
        ("192.43", "NaN"),
        ("192.43", "Infinity"),
        ("192.43", "-Infinity"),
        ("0", "0"),
    ];
    for (ground_truth, answer) in cases {
        let value = score_answer("", ground_truth, answer);
        assert!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "({ground_truth:?}, {answer:?}) gave {value}, which is outside [0, 1]"
        );
    }
}

#[test]
fn the_infinity_and_nan_words_are_not_numbers() {
    // A text that names a non-finite value must never reach the
    // numeric path. It is an ordinary word with no overlap.
    assert_eq!(score_answer("", "192.43", "NaN"), 0.0);
    assert_eq!(score_answer("", "192.43", "Infinity"), 0.0);
    assert_eq!(score_answer("", "192.43", "-Infinity"), 0.0);
}

#[test]
fn a_blank_answer_scores_exactly_zero_for_every_ground_truth() {
    for ground_truth in ["", "192.43", "malicious", "34.7 C", "N/A"] {
        assert_eq!(
            score_answer("", ground_truth, ""),
            0.0,
            "a blank answer against {ground_truth:?} did not score exactly 0.0"
        );
        assert_eq!(score_answer("", ground_truth, "  \t\n "), 0.0);
    }
}
