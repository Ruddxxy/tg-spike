//! These tests cover the one rule the `label` band changes.
//!
//! The whole file is behind the `label` feature. Build it with:
//!
//! ```text
//! cargo test -p eval-script --no-default-features --features label
//! ```
//!
//! A label intent asks for a verdict, a grade, a severity, a language or
//! a translation. The wanted answer is a WORD, and the ground truth
//! often carries a number nobody asked for. In every other band that
//! number makes a number mandatory, so the correct word scores 0.0.
//! This band sends that case to the text comparison.
//!
//! The second half of the file measures what the band COSTS. The band
//! is not free and it must never be registered for an intent whose
//! answer is a quantity.

#![cfg(feature = "label")]

use eval_script::score::score_answer;

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
    // The measured case from the benchmark. In a numeric band the
    // correct words earn 0.0 and the wrong number earns 0.0036, so a
    // wrong answer outranks a right one.
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
    // every numeric rule still applies.
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
// What the band costs
// ---------------------------------------------------------------

#[test]
fn the_label_band_pays_for_the_scaffolding_it_admits() {
    // THIS IS THE PRICE OF THE BAND, stated as a test so that it cannot
    // be forgotten.
    //
    // Dispatch rule 6 is what stops an answer that gives back the words
    // around a value and no value. This band relaxes that rule, so on a
    // truth whose answer IS a quantity the farm comes back and pays
    // 0.667, well above the 0.0831 an honest miner earns 10 percent
    // out.
    //
    // That is why this band is for label intents only: URL_SCAN,
    // SSL_VERIFICATION, CVE_LOOKUP, SENTIMENT_ANALYSIS,
    // TEXT_CLASSIFICATION, CONTENT_MODERATION, FACT_CHECK and
    // LANGUAGE_TRANSLATION. Registering it for a weather, price or gas
    // intent reopens the farm those bands close.
    let farm = score("The temperature was 28.9 C.", "the temperature was C");
    assert!(
        farm > 0.0831,
        "the scaffolding farm earned {farm}; the point of this test is that it DOES pay here"
    );

    // The same answer against the same truth in a numeric band earns
    // 0.0. That assertion lives in `tests/adversarial.rs`, gated the
    // other way.
}
