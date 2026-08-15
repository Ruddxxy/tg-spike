//! This test file covers every scoring branch, A through F.
//!
//! Each branch in the `score` module dispatch has at least one test
//! here. The numeric cases have their own test.

use eval_script::score::score_answer;

/// This helper scores with no question text.
fn score(ground_truth: &str, answer: &str) -> f64 {
    score_answer("", ground_truth, answer)
}

// ---------------------------------------------------------------
// Branch A: both sides numeric, compatible units
// ---------------------------------------------------------------

#[test]
fn branch_a_exact_match_scores_one() {
    assert_eq!(score("192.43", "192.43"), 1.0);
}

#[test]
fn branch_a_trailing_zero_is_the_same_number() {
    assert_eq!(score("192.43", "192.430"), 1.0);
}

#[test]
fn branch_a_a_near_miss_keeps_almost_all_of_its_score() {
    let near = score("192.43", "192.44");
    assert!(
        near > 0.999,
        "one cent out gave {near}, want a score above 0.999"
    );
}

#[test]
fn branch_a_a_wild_answer_scores_almost_nothing() {
    let wild = score("192.43", "999999.99");
    assert!(
        wild < 1e-6,
        "a wild answer gave {wild}, want a score below 0.000001"
    );
}

#[test]
fn branch_a_separates_a_near_miss_from_a_wild_answer() {
    // This is the core improvement claim. The baseline gives 0.0 to
    // both of these. The curve must tell them apart.
    let near = score("192.43", "192.44");
    let wild = score("192.43", "999999.99");
    assert!(
        near > wild * 1000.0,
        "near {near} and wild {wild} are not clearly apart"
    );
}

#[test]
fn branch_a_converts_kelvin_to_celsius() {
    // 307.85 K is 34.7 C. The same temperature in two units must
    // score 1.0.
    assert_eq!(score("34.7 C", "307.85 K"), 1.0);
}

#[test]
fn branch_a_converts_wei_to_gwei() {
    assert_eq!(score("12 gwei", "12000000000 wei"), 1.0);
}

#[test]
fn branch_a_handles_a_ground_truth_of_zero() {
    // A relative error against 0.0 would divide by zero. The absolute
    // floor stops that.
    let exact = score("0", "0");
    assert_eq!(exact, 1.0);
    let close = score("0", "0.01");
    assert!(close.is_finite(), "a zero ground truth gave {close}");
    assert!(
        close > 0.0 && close <= 1.0,
        "a zero ground truth gave {close}"
    );
    let far = score("0", "1000");
    assert!(far < 1e-3, "0 against 1000 gave {far}, want almost nothing");
}

// ---------------------------------------------------------------
// Branch B: both numeric, incompatible units
// ---------------------------------------------------------------

#[test]
fn branch_b_kelvin_against_dollars_scores_zero() {
    assert_eq!(score("307.85 K", "307.85 USD"), 0.0);
}

#[test]
fn branch_b_celsius_against_percent_scores_zero() {
    assert_eq!(score("34.7 C", "34.7%"), 0.0);
}

#[test]
fn branch_b_two_currencies_do_not_convert() {
    // This module has no clock and no market data, so it cannot know
    // a rate. Two currencies are incomparable.
    assert_eq!(score("100 USD", "100 EUR"), 0.0);
}

// ---------------------------------------------------------------
// Branch C: one side declares a unit, the other does not
// ---------------------------------------------------------------

#[test]
fn branch_c_a_format_difference_costs_nothing() {
    // The ground truth and the miner answer come from two different
    // pipelines, and the protocol standardises the miner answer before
    // `rank_answer` sees it. So the miner does not choose whether its
    // unit survives. A penalty would charge every miner for a detail
    // outside its control, and the score must measure only the answer.
    assert_eq!(score("192.43 USD", "192.43 USD"), 1.0);
    assert_eq!(score("192.43 USD", "192.43"), 1.0);
    assert_eq!(score("192.43", "192.43 USD"), 1.0);
    assert_eq!(score("34.7 C", "34.7"), 1.0);
    assert_eq!(score("34.7", "34.7 C"), 1.0);
}

#[test]
fn branch_c_a_bare_number_still_loses_on_a_wrong_value() {
    // Charging nothing for a missing unit must not soften a wrong
    // value. The number itself still decides the score.
    let wrong = score("34.7 C", "307.85");
    assert!(wrong < 1e-3, "a bare but wrong value earned {wrong}");
}

#[test]
fn branch_c_does_not_open_a_spoofing_hole() {
    // Dropping the unit penalty must not let a miner escape the unit
    // check by stating the WRONG unit. When both sides state a unit,
    // the unit is still read and still decides the outcome.
    //
    // A spoof inside ONE family runs through the curve, so it gives a
    // tiny value and not an exact 0.0. The curve never reaches 0.0 for
    // a finite error; that is the property that separates a near miss
    // from a wild answer.
    let spoof = score("34.7 C", "307.85 C");
    assert!(spoof < 1e-3, "the Celsius spoof earned {spoof}");

    // Two families do not compare at all, so this one is an exact 0.0.
    assert_eq!(score("100 USD", "100 EUR"), 0.0);
}

// ---------------------------------------------------------------
// Branch D: exactly one side parses as a number
// ---------------------------------------------------------------

#[test]
fn branch_d_finds_a_number_inside_a_sentence() {
    let found = score("192.43", "the price is 192.43 right now");
    assert!(
        found > 0.9,
        "a sentence holding the right number gave {found}"
    );
}

#[test]
fn branch_d_charges_for_every_extra_number() {
    // A miner that lists many numbers hopes one matches. The score
    // divides by the count of distinct numbers, so listing does not
    // pay.
    let one = score("192.43", "the price is 192.43 right now");
    let many = score("192.43", "192.43 100 200 300 400 500 600 700");
    assert!(
        many < one / 4.0,
        "listing many numbers gave {many}, which is not far enough below {one}"
    );
}

// ---------------------------------------------------------------
// Branch E: neither side parses, text comparison
// ---------------------------------------------------------------

#[test]
fn branch_e_identical_text_scores_one() {
    assert_eq!(score("malicious", "malicious"), 1.0);
}

#[test]
fn branch_e_normalises_case_and_punctuation() {
    assert_eq!(score("Malicious.", "malicious"), 1.0);
}

#[test]
fn branch_e_unrelated_text_scores_zero() {
    assert_eq!(score("malicious", "sunny"), 0.0);
}

/// This test is named after the attack it stops.
///
/// The baseline divides the shared token count by the MINER token
/// count. The answer "is" shares one token with "is malicious" and
/// holds one token, so the baseline gives 1.0 for a word that carries
/// no information. This module divides by the union instead.
#[test]
fn the_is_attack_does_not_score_one() {
    let attack = score("is malicious", "is");
    assert!(
        attack < 1.0,
        "the 'is' attack still scores {attack}, want a score below 1.0"
    );
    // One shared token out of two total tokens.
    assert!(
        (attack - 0.5).abs() < 1e-9,
        "the 'is' attack gave {attack}, want 0.5"
    );
}

#[test]
fn the_is_attack_fades_against_a_longer_ground_truth() {
    // The union divisor grows with the ground truth, so a single
    // common word earns less as the ground truth grows.
    let short = score("is malicious", "is");
    let long = score("is a malicious and dangerous binary file", "is");
    assert!(long < short, "long {long} must be below short {short}");
}

#[test]
fn branch_e_a_subset_of_ground_truth_cannot_reach_one() {
    let subset = score("high risk malicious binary", "malicious");
    assert!(
        subset < 0.3,
        "a one word subset gave {subset}, want well below 1.0"
    );
}

// ---------------------------------------------------------------
// Branch F: negation
// ---------------------------------------------------------------

#[test]
fn branch_f_negation_inverts_rather_than_part_credits() {
    // The baseline gives 0.5 here, because "malicious" is shared.
    // The answer states the opposite of the ground truth, so it is
    // wrong, not half right.
    assert_eq!(score("malicious", "not malicious"), 0.0);
}

#[test]
fn branch_f_matching_negation_still_scores() {
    assert_eq!(score("not malicious", "not malicious"), 1.0);
}

#[test]
fn branch_f_double_negation_cancels() {
    // Two negations return the original meaning.
    assert_eq!(score("malicious", "not not malicious"), 1.0);
}

#[test]
fn branch_f_negation_the_other_way_round() {
    assert_eq!(score("not malicious", "malicious"), 0.0);
}

// ---------------------------------------------------------------
// Hard requirements
// ---------------------------------------------------------------

#[test]
fn a_blank_answer_scores_exactly_zero() {
    assert_eq!(score("192.43", ""), 0.0);
    assert_eq!(score("192.43", "   "), 0.0);
    assert_eq!(score("192.43", "\t\n\r "), 0.0);
}

#[test]
fn every_score_sits_inside_the_closed_range() {
    let cases = [
        ("192.43", "192.43"),
        ("192.43", "-999999999"),
        ("", ""),
        ("malicious", "not malicious"),
        ("0", "0"),
        ("1e308", "-1e308"),
        ("sunny", "42"),
    ];
    for (ground_truth, answer) in cases {
        let value = score(ground_truth, answer);
        assert!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "score({ground_truth:?}, {answer:?}) = {value}, which is outside [0, 1]"
        );
    }
}

#[test]
fn a_junk_question_does_not_change_a_good_score() {
    // Real traffic carries a question like this one.
    let with_junk = score_answer("[direct] 207 -> /price", "192.43", "192.43");
    let without = score_answer("", "192.43", "192.43");
    assert_eq!(with_junk, without);
    assert_eq!(with_junk, 1.0);
}

#[test]
fn copying_the_question_back_scores_zero() {
    let copied = score_answer(
        "what is the weather in tokyo",
        "sunny",
        "what is the weather in tokyo",
    );
    assert_eq!(
        copied, 0.0,
        "an answer that repeats the question must score 0.0"
    );
}
