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
    // "Near" has to mean something in the band being built, so the miss
    // is ONE HUNDREDTH of the band's own tolerance. The curve gives
    // t^2 / (t^2 + (t/100)^2) = 10000/10001 = 0.99990 for that error
    // whatever t is, so the bar below is the same statement in every
    // band rather than a number that happens to hold in one.
    //
    // A fixed miss cannot do that. This test used to score "192.44"
    // against "192.43" and require 0.999. One cent is t/577 at the
    // weather band and t/38 at the price band, so the same two strings
    // asked for very different things: weather cleared the bar by
    // 0.000997 and price by 0.000325. At t = 0.001 price would fail on
    // a scorer that was working correctly.
    //
    // The one-cent case itself is not lost. `golden_vectors.json` pins
    // it by exact f32 bit pattern as `numeric_one_cent_out`, which is a
    // stronger check than this one and is calibrated for t = 0.03.
    let truth = 192.43_f64;
    let miss = truth * (1.0 + eval_script::score::TOLERANCE / 100.0);
    let near = score("192.43", &format!("{miss}"));
    assert!(
        near > 0.9999,
        "a miss of one hundredth of the tolerance gave {near}, want above 0.9999"
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

// Registration runs a structural check that compares a SELF-match, one
// ground truth against its own matching answer, with a CROSS-match, the
// same ground truth against an unrelated answer. The self-match must
// beat the cross-match. Two defects made a self-match score 0.0 or
// below 1.0, and both are pinned here.

#[test]
fn a_text_that_holds_no_token_matches_itself() {
    // "..." is all separators and "none" is a negation marker, so both
    // tokenize to an empty set. An empty set against an empty set is
    // agreement, not a total miss.
    for text in ["...", "*", "none", "---", "!!!"] {
        let earned = score_answer("", text, text);
        assert_eq!(
            earned, 1.0,
            "a ground truth of {text:?} must match itself, earned {earned}"
        );
    }
}

#[test]
fn an_answer_equal_to_the_ground_truth_always_scores_one() {
    // A ground truth holding several numbers used to score its own text
    // below 1.0, because the anti-spray divisor counts the numbers in
    // the answer. A JSON truth earned 0.5 and a CVE line earned 0.333.
    for truth in [
        r#"{"temperature_2m":28.9,"time":"2026-08-10T12:00"}"#,
        "CVE-2021-44228 affects log4j",
        "the 2024 election result is certified",
        "192.43 USD",
        "high risk malicious binary",
        "0",
    ] {
        let earned = score_answer("", truth, truth);
        assert_eq!(
            earned, 1.0,
            "a ground truth of {truth:?} must match itself, earned {earned}"
        );
    }
}

#[test]
fn a_self_match_beats_an_unrelated_cross_match() {
    // The shape of the registration check itself.
    // The third field is the concept group. Two fixtures that encode the
    // SAME value are not a cross-match, so they never pair here.
    let fixtures = [
        ("192.43", "192.43", "price"),
        ("34.7 C", "34.7 C", "body temp"),
        (r#"{"temperature_2m":28.9}"#, "28.9", "air temp"),
        ("The temperature was 28.9 C.", "28.9", "air temp"),
        (
            "high risk malicious binary",
            "high risk malicious binary",
            "verdict",
        ),
        (
            "clear sky no precipitation",
            "clear sky no precipitation",
            "sky",
        ),
        ("none", "none", "empty"),
    ];
    for (index, (truth, matched, group)) in fixtures.iter().enumerate() {
        let self_match = score_answer("", truth, matched);
        assert!(
            self_match > 0.0,
            "self-match for {truth:?} scored {self_match}, which fails registration"
        );
        for (_, unrelated, other_group) in fixtures.iter() {
            if other_group == group {
                continue;
            }
            let cross = score_answer("", truth, unrelated);
            assert!(
                self_match > cross,
                "fixture {index}: self-match {self_match} for {truth:?} did not beat \
                 cross-match {cross} against {unrelated:?}"
            );
        }
    }
}

/// The ground truths whose self match used to depend on byte equality.
///
/// Each one carries more than one number, so the anti-spray divisor
/// charged an answer that repeated the whole truth. The exact-match
/// short circuit hid that, and the short circuit needs BYTE equality.
const MULTI_NUMBER_TRUTHS: [&str; 5] = [
    "CVE-2021-44228 has a severity rating of CRITICAL.",
    "INVOICE 2024-001",
    "{\"temperature_2m\":28.9,\"wind_speed_10m\":11.2}",
    "The high is 31.5 C and the low is 22.4 C.",
    "Partly true. The programme reduced transmission by 40% over 3 years.",
];

#[test]
fn a_self_match_does_not_need_byte_equality() {
    // The node builds the answer of a self-match check, and nothing
    // here controls what it does to the text on the way. Every one of
    // these carries the same words and the same numbers as the truth,
    // so every one must score the same 1.0.
    for truth in MULTI_NUMBER_TRUTHS {
        let doubled_space = match truth.find(' ') {
            Some(at) => format!("{} {}", &truth[..at], &truth[at..]),
            None => truth.to_string(),
        };
        let variants = [
            doubled_space,
            format!("{truth}\n"),
            format!("  {truth}  "),
            format!("{truth}\t"),
        ];
        for variant in variants {
            let earned = score_answer("", truth, &variant);
            assert_eq!(
                earned, 1.0,
                "the truth {truth:?} scored {earned} against its own text {variant:?}"
            );
        }
    }
}

#[test]
fn the_short_circuit_is_not_what_makes_a_self_match_pass() {
    // The same check without any whitespace trick: an answer that holds
    // the truth's numbers and nothing else must reach the 0.75 floor
    // through the RULES, not through the short circuit. The extra full
    // stop keeps the two texts from being equal after trim.
    for truth in MULTI_NUMBER_TRUTHS {
        let restated = format!("{truth}.");
        let earned = score_answer("", truth, &restated);
        assert!(
            earned >= 0.75,
            "the truth {truth:?} scored {earned} against a restatement, want 0.75 or more"
        );
    }
}

#[test]
fn a_spray_still_pays_for_every_wrong_guess() {
    // Five numbers with one right. All five are guesses and the divisor
    // is five.
    //
    // This briefly read 0.250. The quoting rule that lets a restatement
    // keep its self-match stopped charging for a number the truth also
    // holds, and a spray that includes the right number got that number
    // free. The rule now applies only to an answer that adds NOTHING of
    // its own, which a spray does by definition, so the concession is
    // gone and the spray pays the full divisor again.
    let sprayed = score_answer("", "192.43", "192.43 100 200 300 400");
    assert!(
        (sprayed - 0.2).abs() < 1e-12,
        "the spray earned {sprayed}, want 0.2"
    );

    // A spray that holds nothing the truth holds pays for all of them.
    let missed = score_answer("", "192.43", "100 200 300 400 500");
    assert!(
        missed < 1e-9,
        "a spray of wrong numbers earned {missed}, want almost nothing"
    );
}

/// The `label` band relaxes dispatch rule 6 on purpose. See
/// `tests/label_band.rs` for what that band pays here.
#[cfg(not(any(feature = "label", feature = "metadata")))]
#[test]
fn the_unit_only_farm_still_scores_zero() {
    // The two fixes above must not pay a miner that gives back the
    // scaffolding of the ground truth without the value.
    for (truth, farm) in [
        ("192.43 USD", "USD"),
        ("34.7 C", "C"),
        ("12 gwei", "gwei"),
        ("The temperature was 28.9 C.", "the temperature was C"),
        ("The temperature was 28.9 C.", "temperature"),
    ] {
        let earned = score_answer("", truth, farm);
        assert_eq!(
            earned, 0.0,
            "the farm {farm:?} against {truth:?} earned {earned}, want 0.0"
        );
    }
}

#[test]
fn quoting_part_of_the_truth_does_not_buy_the_rest() {
    // The quoting rule pays only an answer that gives back EVERY
    // quantity the truth holds. An answer that keeps the identifier and
    // changes the value has guessed, and it pays for both numbers.
    let right = score_answer("", "INVOICE 2024-001", "INVOICE 2024-001");
    let wrong = score_answer("", "INVOICE 2024-001", "INVOICE 2024-002");
    assert_eq!(right, 1.0, "the right invoice earned {right}");
    assert!(
        wrong < right,
        "the wrong invoice earned {wrong}, level with the right one at {right}"
    );
    assert!(
        (wrong - 0.5).abs() < 1e-12,
        "the wrong invoice earned {wrong}, want 0.5"
    );

    // The same shape without an identifier.
    let partly = score_answer(
        "",
        "Order 12345 shipped 3 items",
        "Order 12345 shipped 9 items",
    );
    assert!(
        partly < 0.75,
        "echoing the order number with a wrong count earned {partly}"
    );
}
