//! These tests cover the one rule the `metadata` band changes.
//!
//! The whole file is behind the `metadata` feature. Build it with:
//!
//! ```text
//! cargo test -p eval-script --no-default-features --features metadata
//! ```
//!
//! This band serves the same intents the `label` band does and answers
//! the same complaint: a ground truth that carries a confidence beside
//! a verdict, or a CVSS beside a severity, made a number mandatory and
//! killed the correct word.
//!
//! It differs in what it pays. `label` pays the full text score, which
//! reopens the scaffolding farm at 0.6667. This band pays
//! `RULE6_ATTENUATION` of that score.
//!
//! The second half of the file is the reason the band exists: every
//! farm the numeric bands close must stay closed here, priced under the
//! honest bar rather than at exactly zero. That is a WEAKER guarantee
//! than the numeric bands hold, and these tests are where the weaker
//! guarantee is written down.

#![cfg(feature = "metadata")]

use eval_script::score::{score_answer, RULE6_ATTENUATION};

/// What an honest miner earns when it gives a real number and is 10
/// percent out at this band's tolerance of 0.03.
///
/// Every farm in this file must stay under it. This is the bar that
/// replaces the exact-zero guarantee the numeric bands hold.
const HONEST_BAR: f64 = 0.0826;

/// This helper scores with no question text.
fn score(ground_truth: &str, answer: &str) -> f64 {
    score_answer("", ground_truth, answer)
}

/// The rows of the promotion benchmark that scored 0.0 for BOTH
/// candidates in the numeric bands, as (truth, good answer, bad answer).
const PREVIOUSLY_TIED: [(&str, &str, &str); 6] = [
    (
        "{\"verdict\":\"phishing\",\"confidence\":0.97}",
        "phishing",
        "benign",
    ),
    ("{\"grade\":\"A\",\"protocol\":\"TLS 1.3\"}", "A", "F"),
    (
        "CVE-2021-44228 has a severity rating of CRITICAL.",
        "CRITICAL",
        "MEDIUM",
    ),
    (
        "{\"cve\":\"CVE-2021-44228\",\"severity\":\"critical\",\"cvss\":9.8}",
        "critical",
        "low",
    ),
    (
        "{\"label\":\"negative\",\"score\":0.88}",
        "negative",
        "positive",
    ),
    ("{\"verdict\":\"false\",\"sources\":3}", "false", "true"),
];

#[test]
fn a_label_answer_beats_a_wrong_one_where_the_numeric_bands_tie() {
    for (truth, good, bad) in PREVIOUSLY_TIED {
        let earned = score(truth, good);
        let wrong = score(truth, bad);
        assert!(
            earned > 0.0,
            "the correct label {good:?} against {truth:?} earned {earned}"
        );
        assert!(
            earned > wrong,
            "the correct label {good:?} earned {earned} and the wrong {bad:?} earned {wrong}"
        );
    }
}

#[test]
fn a_correct_verdict_beats_a_wrong_number() {
    // This row is what sets the BOTTOM of the attenuation window. The
    // correct words carry no quantity and are attenuated. The wrong
    // "60%" carries one, so it reaches the numeric branch and is not
    // attenuated at all. The band works only while the first still
    // outscores the second.
    let truth = "Partly true. The programme reduced transmission by 40%.";
    let words = score(truth, "partly true");
    let wrong_number = score(truth, "60%");
    assert!(
        words > wrong_number,
        "the correct words earned {words} and the wrong number earned {wrong_number}"
    );
}

#[test]
fn a_number_that_is_asked_for_is_still_scored_as_a_number() {
    // The band changes ONE rule. When the answer does carry a quantity,
    // every numeric rule still applies and no attenuation touches it.
    assert_eq!(score("9.8", "9.8"), 1.0);
    assert_eq!(score("100 USD", "100 EUR"), 0.0);
    let near = score("192.43", "192.44");
    assert!(near > 0.999, "a near miss earned {near}");
    let wild = score("192.43", "999999.99");
    assert!(wild < 1e-6, "a wild answer earned {wild}");
}

#[test]
fn the_negation_rule_still_holds() {
    assert_eq!(score("The URL is not malicious.", "malicious"), 0.0);
}

#[test]
fn a_blank_answer_still_scores_exactly_zero() {
    for blank in ["", " ", "\t\n"] {
        assert_eq!(score("{\"verdict\":\"false\",\"sources\":3}", blank), 0.0);
    }
}

#[test]
fn copying_the_question_back_still_scores_zero() {
    let question = "[direct] 207 -> /price";
    assert_eq!(score_answer(question, "malicious", question), 0.0);
}

// ---------------------------------------------------------------
// What the band costs, and the farms it must still close
// ---------------------------------------------------------------

#[test]
fn the_scaffolding_farm_stays_under_the_honest_bar() {
    // THIS IS THE PRICE OF THE BAND, stated as a test so that it cannot
    // be forgotten.
    //
    // In a numeric band this answer earns EXACTLY 0.0, and no constant
    // has to be right for that to hold. Here it earns something. The
    // guarantee is therefore quantitative rather than structural: the
    // farm pays less than an honest miner PROVIDED the attenuation
    // constant is correct.
    //
    // In the `label` band the same answer earns 0.6667, which is eight
    // times the honest bar. That is the farm this band closes.
    let farm = score("The temperature was 28.9 C.", "the temperature was C");
    assert!(
        farm > 0.0,
        "the farm earned {farm}; it is attenuated, not zeroed"
    );
    assert!(
        farm < HONEST_BAR,
        "the scaffolding farm earned {farm}, at or above the honest bar {HONEST_BAR}"
    );
}

#[test]
fn every_measured_farm_stays_under_the_honest_bar() {
    // The farm rows from `corpus-eval`'s rule6_probe, including the
    // three ceiling rows, whose answer is the whole truth with the
    // number taken out. The last is the largest score the text branch
    // was measured to pay a no-quantity answer.
    for (truth, answer) in [
        ("The temperature was 28.9 C.", "the temperature was C"),
        ("The temperature was 28.9 C.", "The temperature was C."),
        ("The temperature was 28.9 C.", "the temperature"),
        ("The temperature was 28.9 C.", "temperature"),
        ("The temperature was 28.9 C.", "C"),
        ("The price is 192.43 USD.", "the price is USD"),
        ("12 gwei", "gwei"),
        ("Temperature was 28.9 C", "temperature"),
        (
            "{\"temperature_2m\":28.9,\"time\":\"2026-08-10T12:00\"}",
            "temperature and time",
        ),
        ("{\"temperature_2m\":28.9,\"city\":\"Tokyo\"}", "Tokyo"),
        (
            "The station at the site reported that the outdoor air temperature was 28.9 C",
            "the station at site reported that outdoor air temperature was C",
        ),
        (
            "The weather station on the north side of the airport reported that the \
             outdoor air temperature measured at two metres above ground level was 28.9 C",
            "The weather station on the north side of the airport reported that the \
             outdoor air temperature measured at two metres above ground level was C",
        ),
    ] {
        let earned = score(truth, answer);
        assert!(
            earned < HONEST_BAR,
            "the farm answer {answer:?} against {truth:?} earned {earned}, \
             at or above the honest bar {HONEST_BAR}"
        );
    }
}

#[test]
fn the_unit_only_farm_stays_closed() {
    // The numeric bands give these exactly 0.0. Here they are
    // attenuated, and the requirement is the honest bar rather than
    // zero.
    for (truth, answer) in [
        ("192.43 USD", "USD"),
        ("34.7 C", "C"),
        ("307.85 K", "K"),
        ("15 %", "%"),
        ("12 gwei", "gwei"),
        ("$192.43", "USD"),
        ("192.43 USD", "USD USD USD USD USD USD USD USD"),
    ] {
        let earned = score(truth, answer);
        assert!(
            earned < HONEST_BAR,
            "the unit-only answer {answer:?} against {truth:?} earned {earned}"
        );
    }
}

#[test]
fn the_attenuation_leaves_room_above_and_below() {
    // The constant is the geometric mean of an open window. These two
    // assertions are the walls of that window, so a later edit that
    // moves the constant out of it fails here rather than in
    // production.
    //
    // Above: the largest text score measured for a no-quantity answer
    // is 0.9167, and it climbs towards 1.0 as the truth grows. Taking
    // the ceiling as 1.0 outright, the attenuation alone must sit under
    // the honest bar.
    assert!(
        RULE6_ATTENUATION < HONEST_BAR,
        "attenuation {RULE6_ATTENUATION} is not under the honest bar {HONEST_BAR}, \
         so a long enough truth would let a farm reach it"
    );
    // Below: the tightest label pair needs the attenuated 0.25 to beat
    // the unattenuated 0.0036.
    assert!(
        RULE6_ATTENUATION * 0.25 > 0.0036,
        "attenuation {RULE6_ATTENUATION} is too small for the FACT_CHECK row"
    );
}
