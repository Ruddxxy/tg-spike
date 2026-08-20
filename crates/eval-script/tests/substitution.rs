//! These tests cover the rule that charges a substitution more than an
//! omission.
//!
//! Every ground truth here holds no quantity, so every case reaches the
//! text branch in all four bands and none of these tests is gated.
//!
//! The attack: the answer gives back the truth's wording and changes
//! the one word that carries the meaning. Against
//! `"Paris is the capital of France."` the wrong
//! `"Lyon is the capital of France."` shared five tokens of seven and
//! scored 0.7143, while the correct `"Paris"` shared one of six and
//! scored 0.1667. The wrong answer earned four times the right one.
//!
//! Under a promotion bar where a tie is a loss, a row like this is not
//! a weak score. It is a rejection.

use eval_script::score::score_answer;

/// This helper scores with no question text.
fn score(ground_truth: &str, answer: &str) -> f64 {
    score_answer("", ground_truth, answer)
}

/// The four benchmark rows that inverted in every band, as
/// (truth, correct answer, wrong answer).
const SUBSTITUTIONS: [(&str, &str, &str); 4] = [
    (
        "The scan verdict for this URL is malicious.",
        "malicious",
        "The scan verdict for this URL is clean.",
    ),
    (
        "The sentiment of the review is positive.",
        "positive",
        "The sentiment of the review is negative.",
    ),
    ("The claim is false.", "false", "The claim is true."),
    (
        "Paris is the capital of France.",
        "Paris",
        "Lyon is the capital of France.",
    ),
];

#[test]
fn a_short_exact_answer_beats_a_long_substitution() {
    for (truth, good, bad) in SUBSTITUTIONS {
        let earned = score(truth, good);
        let swapped = score(truth, bad);
        assert!(
            earned > swapped,
            "against {truth:?} the correct {good:?} earned {earned} \
             and the substitution {bad:?} earned {swapped}"
        );
    }
}

#[test]
fn padding_a_substitution_never_helps_the_miner() {
    // THIS IS WHY THE CHARGE READS RECALL AND NOT THE OVERLAP SCORE.
    //
    // The charge has to be blind to the answer's length. A charge of
    // `1 - overlap` would let this attack climb to 0.148 at twelve
    // filler words, which beats the 0.125 the correct answer earns.
    // Recall does not move when the miner adds junk, so every filler
    // word here makes the attack strictly worse.
    let truth = "The scan verdict for this URL is malicious.";
    let honest = score(truth, "malicious");
    let bare_attack = score(truth, "The scan verdict for this URL is clean.");

    let mut padded = String::from("The scan verdict for this URL is clean.");
    let mut previous = bare_attack;
    for index in 0..20 {
        padded.push_str(" filler");
        padded.push_str(&index.to_string());
        let earned = score(truth, &padded);
        assert!(
            earned <= previous,
            "padding to {} words raised the attack from {previous} to {earned}",
            index + 1
        );
        assert!(
            earned < honest,
            "a substitution padded with {} words earned {earned}, \
             above the honest {honest}",
            index + 1
        );
        previous = earned;
    }
}

#[test]
fn leaving_a_token_out_is_not_charged() {
    // An omission is not a false claim. An answer that is a subset of
    // the ground truth asserts nothing the truth denies, so the rule
    // must not touch it. These are the farm rows: they are held where
    // they were, not lowered, because lowering them here is what a
    // stopword list would do and that reopens other doors.
    assert_eq!(score("is malicious", "is"), 0.5);
    assert_eq!(score("high risk malicious binary", "malicious"), 0.25);

    // The scaffolding farm reaches the text branch only in the `label`
    // band; dispatch rule 6 gives it 0.0 everywhere else. Either way
    // this rule must leave it exactly where it found it, because the
    // answer asserts nothing foreign.
    let scaffolding = score("The temperature was 28.9 C.", "the temperature was C");
    #[cfg(feature = "label")]
    assert!(
        (scaffolding - 0.6666666666666666).abs() < 1e-12,
        "the scaffolding farm earned {scaffolding}, want the documented 0.667"
    );
    #[cfg(not(any(feature = "label", feature = "metadata")))]
    assert_eq!(
        scaffolding, 0.0,
        "dispatch rule 6 gives this 0.0 outside the label band"
    );
}

#[test]
fn giving_back_everything_and_more_is_not_a_substitution() {
    // An elaboration leaves nothing out, so nothing was replaced. The
    // answer holds every token of the truth and adds its own.
    assert_eq!(score("malicious", "definitely malicious"), 0.5);
    let padded = score("malicious", "malicious filler filler filler");
    assert_eq!(padded, 0.5, "a repeated filler adds one distinct token");
}

#[test]
fn a_self_match_is_untouched() {
    for (truth, _, _) in SUBSTITUTIONS {
        assert_eq!(score(truth, truth), 1.0, "self match on {truth:?}");
        // The same words with the spacing changed still score 1.0, so
        // the rule does not depend on the exact-match short circuit.
        let spaced = truth.replacen(' ', "  ", 1);
        assert_eq!(score(truth, &spaced), 1.0, "spaced self match on {truth:?}");
    }
}

#[test]
fn the_rule_can_only_lower_a_score() {
    // This is the property that makes the rule safe: it multiplies the
    // union score by a factor that is never above 1.0, so it cannot pay
    // a farm the union divisor already closed. A rule that can only
    // subtract cannot open a door.
    let cases = [
        ("is malicious", "is"),
        ("high risk malicious binary", "malicious"),
        ("The temperature was 28.9 C.", "the temperature was C"),
        ("Paris is the capital of France.", "Lyon is the capital"),
        ("The claim is false.", "The claim is true."),
        ("malicious", "definitely malicious"),
    ];
    for (truth, answer) in cases {
        let earned = score(truth, answer);
        let union = eval_script::text::overlap_score(
            &eval_script::text::tokenize(truth),
            &eval_script::text::tokenize(answer),
        );
        assert!(
            earned <= union + 1e-12,
            "{answer:?} against {truth:?} earned {earned}, above the union score {union}"
        );
    }
}

#[test]
fn a_wrong_word_in_a_short_truth_still_loses() {
    // The shortest possible substitution: a two token truth with one
    // token swapped. The rule has to hold at the small end too.
    let truth = "verdict malicious";
    let good = score(truth, "malicious");
    let bad = score(truth, "verdict clean");
    assert!(
        good > bad,
        "the correct {good} did not beat the substitution {bad}"
    );
}
