//! This module holds a native copy of the protocol's baseline scorer.
//!
//! The copy exists so that a table can show the baseline score beside
//! our score without a wasm call for every row. The wazero runner
//! scores the same rows through the REAL reference module, and the
//! `baseline_matches_the_reference_module` check in the evaluation
//! pipeline compares the two. If this copy ever drifts from the real
//! module, that check fails.
//!
//! The source is `wasm-scoring-module/rust-module/src/lib.rs` in
//! github.com/telegraphprotocol/telegraph-examples:
//!
//! ```text
//! fn score(ground_truth: &str, miner_answer: &str) -> f32 {
//!     if miner_answer == ground_truth {
//!         return 1.0;
//!     }
//!     word_overlap(miner_answer, ground_truth)
//! }
//! ```
//!
//! `word_overlap` lowercases both sides, splits on whitespace, counts
//! the answer tokens that appear in the ground truth, and divides by
//! the count of ANSWER tokens. Dividing by the answer token count is
//! the defect this submission fixes: it pays a miner for saying less.

/// This function scores one pair the way the reference module does.
///
/// A high score is good. The range is 0.0 to 1.0.
///
/// The function returns 0.0 for a blank answer, which matches the
/// early return in the reference `rank_answer`.
pub fn baseline_score(ground_truth: &str, miner_answer: &str) -> f64 {
    if miner_answer.trim().is_empty() {
        return 0.0;
    }
    if miner_answer == ground_truth {
        return 1.0;
    }
    word_overlap(miner_answer, ground_truth)
}

/// This function counts the shared words and divides by the ANSWER
/// word count.
///
/// The divisor is the miner answer token count, exactly as the
/// reference module does it.
fn word_overlap(miner_answer: &str, ground_truth: &str) -> f64 {
    let truth_words: Vec<String> = ground_truth
        .split_whitespace()
        .map(|word| word.to_lowercase())
        .collect();
    let answer_words: Vec<String> = miner_answer
        .split_whitespace()
        .map(|word| word.to_lowercase())
        .collect();

    if answer_words.is_empty() {
        return 0.0;
    }

    let mut shared = 0usize;
    for word in &answer_words {
        if truth_words.contains(word) {
            shared += 1;
        }
    }

    // The counts are small, so this conversion is exact.
    (shared as f64) / (answer_words.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_match_scores_one() {
        assert_eq!(baseline_score("192.43", "192.43"), 1.0);
    }

    #[test]
    fn a_blank_answer_scores_zero() {
        assert_eq!(baseline_score("192.43", ""), 0.0);
        assert_eq!(baseline_score("192.43", "   "), 0.0);
    }

    #[test]
    fn the_is_attack_scores_one_on_the_baseline() {
        // This is the defect. One shared word, one answer word, so the
        // divisor is 1 and the score is 1.0 for a word that carries no
        // information.
        assert_eq!(baseline_score("is malicious", "is"), 1.0);
    }

    #[test]
    fn a_near_miss_number_scores_zero_on_the_baseline() {
        // The baseline has no idea what a number is, so one cent out
        // and a million out both score 0.0.
        assert_eq!(baseline_score("192.43", "192.44"), 0.0);
        assert_eq!(baseline_score("192.43", "999999.99"), 0.0);
    }
}
