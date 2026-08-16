//! This test file attacks THIS module, not the baseline.
//!
//! Each test plays a miner that wants a high score while it gives no
//! information. A test records the score the strategy earns and
//! checks the limit that holds it down.
//!
//! Two tests below record a strategy that STILL WORKS. Those tests
//! assert the profit, so that a later edit cannot quietly change the
//! weakness without a test failing. A weakness with a test on it is a
//! known cost. A weakness with no test is a surprise.

use eval_script::score::score_answer;

/// This helper scores with no question text.
fn score(ground_truth: &str, answer: &str) -> f64 {
    score_answer("", ground_truth, answer)
}

/// This is the score an honest miner earns when it is slightly wrong.
///
/// Every attack must stay below this number, or the attack pays
/// better than honest work. A 1 percent error is a realistic honest
/// miss.
fn honest_but_imperfect() -> f64 {
    score("192.43", "194.35")
}

#[test]
fn honest_but_imperfect_earns_a_useful_score() {
    let honest = honest_but_imperfect();
    assert!(
        honest > 0.05,
        "an honest miner one percent out earns {honest}, which is too low to be worth the work"
    );
}

// ---------------------------------------------------------------
// Constant answers
// ---------------------------------------------------------------

#[test]
fn a_constant_word_does_not_beat_honest_work() {
    // A miner answers the same common word to every question.
    let cases = ["yes", "the", "true", "sunny"];
    for answer in cases {
        let earned = score("192.43", answer);
        assert_eq!(
            earned, 0.0,
            "the constant word {answer:?} earned {earned} against a numeric ground truth"
        );
    }
}

#[test]
fn a_constant_number_does_not_beat_honest_work() {
    // A miner answers the same number to every question. It wins only
    // when the ground truth happens to sit near that number.
    let answer = "100";
    let mut total = 0.0;
    let truths = ["192.43", "34.7", "0.5", "999", "12"];
    for truth in truths {
        total += score(truth, answer);
    }
    let average = total / (truths.len() as f64);
    assert!(
        average < honest_but_imperfect(),
        "a constant number averaged {average}, which beats honest work"
    );
}

// ---------------------------------------------------------------
// Subset of ground truth
// ---------------------------------------------------------------

#[test]
fn a_subset_of_the_ground_truth_loses_score_for_what_it_omits() {
    let full = score("high risk malicious binary", "high risk malicious binary");
    let subset = score("high risk malicious binary", "malicious");
    assert_eq!(full, 1.0);
    assert!(
        subset < 0.3,
        "a one word subset earned {subset}, which is too close to the full answer"
    );
}

// ---------------------------------------------------------------
// Empty, whitespace, control characters
// ---------------------------------------------------------------

#[test]
fn blank_and_control_answers_earn_nothing() {
    assert_eq!(score("192.43", ""), 0.0);
    assert_eq!(score("192.43", "    "), 0.0);
    assert_eq!(score("192.43", "\t\n\r"), 0.0);
    // A control character is not whitespace, but it is not a token
    // either.
    let control = score("192.43", "\u{0}\u{1}\u{2}");
    assert_eq!(control, 0.0, "control characters earned {control}");
}

// ---------------------------------------------------------------
// Long answers
// ---------------------------------------------------------------

#[test]
fn the_score_does_not_grow_with_answer_length() {
    // A miner pads its answer, hoping that length pays.
    let short = score("malicious", "malicious");
    let mut padded = String::from("malicious");
    for index in 0..200 {
        padded.push_str(" filler");
        padded.push_str(&index.to_string());
    }
    let long = score("malicious", &padded);
    assert!(
        long < short,
        "padding raised the score from {short} to {long}"
    );
    assert!(long < 0.05, "a padded answer still earned {long}");
}

#[test]
fn a_very_long_answer_stays_bounded_and_cheap() {
    // The token cap bounds the work. The score must stay in range and
    // must not reward the padding.
    let mut huge = String::new();
    for index in 0..5000 {
        huge.push_str("token");
        huge.push_str(&index.to_string());
        huge.push(' ');
    }
    let earned = score("malicious", &huge);
    assert!(
        earned.is_finite() && (0.0..=1.0).contains(&earned),
        "a huge answer gave {earned}, which is outside the range"
    );
    assert!(earned < 0.05, "a huge answer earned {earned}");
}

// ---------------------------------------------------------------
// Many candidate numbers
// ---------------------------------------------------------------

#[test]
fn listing_many_numbers_does_not_pay() {
    // The miner sprays numbers, hoping one lands.
    let honest = score("192.43", "192.43");
    let spray = score(
        "192.43",
        "1 2 5 10 20 50 100 150 192.43 200 250 300 500 1000",
    );
    assert_eq!(honest, 1.0);
    assert!(
        spray < 0.15,
        "a spray of numbers earned {spray}, which is too close to an honest answer"
    );
    assert!(
        spray < honest_but_imperfect(),
        "a spray of numbers earned {spray}, which beats honest work"
    );
}

// ---------------------------------------------------------------
// Unit spoofing
// ---------------------------------------------------------------

#[test]
fn unit_spoofing_fails_in_both_directions() {
    // The miner returns a Kelvin magnitude but labels it Celsius,
    // hoping the scorer reads only the number.
    let spoof = score("34.7 C", "307.85 C");
    assert!(
        spoof < 1e-3,
        "the spoof earned {spoof}; the scorer trusted the label over the value"
    );
    // The reverse: a Celsius magnitude labelled Kelvin.
    let reverse = score("307.85 K", "34.7 K");
    assert!(reverse < 1e-3, "the reverse spoof earned {reverse}");
}

#[test]
fn a_correct_unit_conversion_is_not_a_spoof() {
    // The honest version of the same pair must still score 1.0.
    assert_eq!(score("34.7 C", "307.85 K"), 1.0);
}

// ---------------------------------------------------------------
// Precision spam
// ---------------------------------------------------------------

#[test]
fn precision_spam_earns_no_more_than_the_plain_answer() {
    let plain = score("192.43", "192.43");
    let spam = score("192.43", "192.4300000000001");
    assert!(
        spam <= plain,
        "precision spam earned {spam}, above the plain answer {plain}"
    );
    // It is the right answer, so it keeps its score. The point is
    // that the extra digits buy nothing.
    assert!(spam > 0.999, "precision spam earned {spam}");
}

// ---------------------------------------------------------------
// Hedging
// ---------------------------------------------------------------

#[test]
fn a_range_answer_pays_the_many_numbers_penalty() {
    // "34 to 36" holds two numbers, so it is not one clean value.
    let range = score("35", "34 to 36");
    let exact = score("35", "35");
    assert_eq!(exact, 1.0);
    assert!(
        range < 0.55,
        "a hedged range earned {range}, which is too close to a committed answer"
    );
}

/// This test records a strategy that STILL WORKS.
///
/// A hedge word in front of a correct number costs the miner nothing.
/// The parser treats "about" as noise and reads the number.
///
/// The judgement here is that this is acceptable. The miner still has
/// to state the right number, so it gives the same information as a
/// bare number. A rule that punished the word would also punish an
/// honest miner whose upstream feed writes "about 42".
#[test]
fn known_weakness_a_hedge_word_is_free() {
    let hedged = score("42", "about 42");
    let plain = score("42", "42");
    assert_eq!(
        hedged, plain,
        "the hedge word changed the score from {plain} to {hedged}"
    );
    assert_eq!(hedged, 1.0);
}

// ---------------------------------------------------------------
// Negation
// ---------------------------------------------------------------

#[test]
fn negation_cannot_farm_partial_credit() {
    // The baseline gives 0.5 here, because the two texts share the
    // word "malicious".
    assert_eq!(score("malicious", "not malicious"), 0.0);
}

#[test]
fn double_negation_does_not_beat_the_plain_answer() {
    let plain = score("malicious", "malicious");
    let doubled = score("malicious", "not not malicious");
    assert!(
        doubled <= plain,
        "double negation earned {doubled}, above the plain answer {plain}"
    );
}

// ---------------------------------------------------------------
// Copying the question
// ---------------------------------------------------------------

#[test]
fn copying_the_question_earns_nothing() {
    let question = "what is the current temperature in tokyo";
    let copied = score_answer(question, "34.7 C", question);
    assert_eq!(copied, 0.0, "the copied question earned {copied}");
}

#[test]
fn a_junk_question_cannot_be_farmed() {
    // Real traffic carries a question with no useful text. A miner
    // that echoes it must not gain from it.
    let question = "[direct] 207 -> /price";
    let echoed = score_answer(question, "192.43", question);
    assert!(echoed < 0.15, "echoing a junk question earned {echoed}");
}

// ---------------------------------------------------------------
// Boundary probing
// ---------------------------------------------------------------

#[test]
fn there_is_no_cliff_to_sit_on() {
    // A threshold rule has a boundary a miner can sit just inside.
    // This curve has no boundary: the score falls without a step, so
    // probing gains nothing. Each step out must lower the score.
    let truth = 100.0f64;
    let mut previous = 2.0f64;
    for step in 0..12 {
        let error = 0.001 * (1.7f64).powi(step);
        let answer = truth + truth * error;
        let earned = score("100", &format!("{answer}"));
        assert!(
            earned < previous,
            "the score did not fall at step {step}: {earned} against {previous}"
        );
        previous = earned;
    }
}

/// This test records a strategy that STILL WORKS.
///
/// A single common token earns 0.5 against a two token ground truth.
/// The union divisor stops the 1.0 the baseline gave, but it cannot
/// push a one-of-two overlap below one half.
///
/// This is the residual cost of a set based overlap. A fix would need
/// a token weight, such as a rarity weight, and that needs a corpus
/// the module cannot carry.
#[test]
fn known_weakness_one_common_token_against_a_short_ground_truth() {
    let earned = score("is malicious", "is");
    assert!(
        (earned - 0.5).abs() < 1e-9,
        "the one token attack earned {earned}, want 0.5"
    );
    // The weakness shrinks as the ground truth grows.
    let longer = score("is a malicious and dangerous file", "is");
    assert!(
        longer < 0.2,
        "against a longer ground truth it earned {longer}"
    );
}

#[test]
fn no_attack_in_this_file_reaches_a_perfect_score() {
    // A sweep over the attacks that do not state the right value.
    // None may reach 1.0.
    let attacks = [
        ("192.43", "yes"),
        ("192.43", ""),
        ("192.43", "   "),
        ("192.43", "1 2 3 4 5 6 7 8 9 10"),
        ("34.7 C", "307.85 C"),
        ("malicious", "not malicious"),
        ("high risk malicious binary", "malicious"),
        ("35", "34 to 36"),
        ("is malicious", "is"),
    ];
    for (truth, answer) in attacks {
        let earned = score(truth, answer);
        assert!(
            earned < 1.0,
            "the attack ({truth:?}, {answer:?}) reached a perfect {earned}"
        );
    }
}

// ---------------------------------------------------------------
// Cross-branch farming
//
// The tests above attack inside one scoring branch. The stronger
// attack crosses branches: the miner sends something that makes the
// scorer LEAVE the numeric path, then farms the text path, which has a
// much softer floor.
//
// A ground truth of "192.43 USD" against an answer of "USD" was
// expected to fall through to text and score 0.5. It does not: the
// numeric branch has no text fallback, so it already scored 0.0. The
// cross-branch probe DID find real leaks on the prose and JSON
// renderings of a ground truth, and the tests below pin those.
// ---------------------------------------------------------------

/// The prose rendering of a real corpus ground truth.
const PROSE_TRUTH: &str = "The temperature was 28.9 C.";

/// The JSON rendering of the same corpus ground truth.
const JSON_TRUTH: &str = "{\"temperature_2m\":28.9,\"time\":\"2026-08-10T12:00\"}";

#[test]
fn an_answer_of_the_unit_alone_earns_nothing() {
    for (truth, answer) in [
        ("192.43 USD", "USD"),
        ("34.7 C", "C"),
        ("307.85 K", "K"),
        ("15 %", "%"),
        ("12 gwei", "gwei"),
        ("$192.43", "USD"),
    ] {
        let earned = score(truth, answer);
        assert_eq!(
            earned, 0.0,
            "the unit-only answer {answer:?} against {truth:?} earned {earned}"
        );
    }
}

#[test]
fn repeating_the_unit_many_times_earns_nothing() {
    let earned = score("192.43 USD", "USD USD USD USD USD USD USD USD");
    assert_eq!(earned, 0.0, "the repeated unit earned {earned}");
}

#[test]
fn giving_back_the_prose_words_without_the_number_earns_nothing() {
    // This is the leak the cross-branch probe found. The answer repeats the
    // scaffolding of the ground truth and omits the value. It scored
    // 0.667 before the fix, while an honest miner 10 percent out
    // scored 0.081.
    for answer in [
        "the temperature was C",
        "temperature",
        "The temperature was",
    ] {
        let earned = score(PROSE_TRUTH, answer);
        assert_eq!(
            earned, 0.0,
            "the scaffolding answer {answer:?} earned {earned}"
        );
    }
}

#[test]
fn a_quantity_ground_truth_needs_a_quantity_answer() {
    // The general rule behind the test above: when the ground truth
    // carries a number, an answer with no number has not answered.
    let earned = score(PROSE_TRUTH, "warm and sunny today");
    assert_eq!(earned, 0.0, "a text answer to a quantity earned {earned}");
}

#[test]
fn the_right_number_still_scores_on_every_rendering() {
    // The fix must not break the honest case. The same correct answer
    // must score the same against all three renderings of one truth.
    for answer in ["28.9", "28.9 C", "302.05 K"] {
        assert_eq!(
            score("28.9", answer),
            1.0,
            "{answer:?} against the bare rendering"
        );
        assert_eq!(
            score(PROSE_TRUTH, answer),
            1.0,
            "{answer:?} against the prose rendering"
        );
        assert_eq!(
            score(JSON_TRUTH, answer),
            1.0,
            "{answer:?} against the JSON rendering"
        );
    }
}

#[test]
fn a_number_inside_a_prose_truth_is_found() {
    // A near miss must still grade, not fall to zero, on every
    // rendering.
    let bare = score("28.9", "28.5");
    let prose = score(PROSE_TRUTH, "28.5");
    let json = score(JSON_TRUTH, "28.5");
    assert!(bare > 0.5, "the bare rendering gave {bare}");
    assert_eq!(bare, prose, "prose {prose} must match bare {bare}");
    assert_eq!(json, bare, "json {json} must match bare {bare}");
}

/// This test pins the fix for a strategy that USED TO WORK.
///
/// A JSON ground truth carries a timestamp, so the lenient scan used to
/// hand back the year, the month, the day, the hour and the minute as
/// candidate match targets, plus the `2` inside the key name
/// `temperature_2m`. Each of those scored 1.0. The truth side now
/// ignores a number that sits inside a quoted string, so the only
/// candidate left is the value in JSON value position.
///
/// The score for a date part must now match the score the SAME answer
/// earns against the bare and prose renderings of the same truth. That
/// equality is the real property: the score must not depend on the
/// rendering, because the miner does not choose the rendering.
#[test]
fn a_json_truth_cannot_be_farmed_with_a_date_or_key_part() {
    for farm in ["2026", "8", "10", "12", "0", "2", "00"] {
        let json = score(JSON_TRUTH, farm);
        let bare = score("28.9", farm);
        let prose = score(PROSE_TRUTH, farm);
        assert!(
            json < 1e-2,
            "the incidental number {farm:?} earned {json} against the JSON truth"
        );
        assert_eq!(
            json, bare,
            "{farm:?} scored {json} against JSON and {bare} against bare"
        );
        assert_eq!(
            json, prose,
            "{farm:?} scored {json} against JSON and {prose} against prose"
        );
    }
}

/// This test pins the registration property the fix restores.
///
/// Registration compares a SELF-match with a CROSS-match and requires
/// the self-match to win strictly. The `12` of `T12:00` used to make the
/// unrelated answer `12 gwei` score 1.0 against the JSON truth, which
/// tied the correct answer `28.9` and failed the check.
#[test]
fn a_json_truth_self_match_beats_an_unrelated_cross_match() {
    let self_match = score(JSON_TRUTH, "28.9");
    assert_eq!(self_match, 1.0, "the correct answer earned {self_match}");
    for unrelated in ["12 gwei", "192.43 USD", "2026", "0"] {
        let cross = score(JSON_TRUTH, unrelated);
        assert!(
            self_match > cross,
            "the cross-match {unrelated:?} earned {cross}, which does not lose to {self_match}"
        );
    }
}

/// This test guards the exception that keeps a quoted real value.
///
/// A rendering that quotes its value, as `{"temperature_2m":"28.9 C"}`
/// does, must still score. The admission test is `parse_value`, so a
/// quoted string that is one clean value keeps its number while a
/// quoted timestamp does not.
#[test]
fn a_quoted_value_in_a_json_truth_still_scores() {
    for quoted in [
        // The whole string is one clean value.
        "{\"temperature_2m\":\"28.9 C\"}",
        "{\"temperature_2m\":\"28.9\"}",
        // The string wraps one value in a sentence. The number still
        // stands on its own, so it is not part of a word.
        "{\"summary\":\"28.9 C in Paris\"}",
    ] {
        assert_eq!(
            score(quoted, "28.9"),
            1.0,
            "the correct answer lost its score against {quoted:?}"
        );
        assert_eq!(
            score(quoted, "28.5"),
            score("28.9", "28.5"),
            "a near miss against {quoted:?} left the bare rendering"
        );
    }
    // The rule must not pay the scaffolding of the string it admits.
    assert_eq!(score("{\"summary\":\"28.9 C in Paris\"}", "summary"), 0.0);
}
