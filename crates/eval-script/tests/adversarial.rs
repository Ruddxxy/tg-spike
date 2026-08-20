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
    // The miss is ONE THIRD of the band's own tolerance, so this bar
    // means the same thing in every band. The curve gives
    // t^2 / (t^2 + (t/3)^2) = 0.9 for that error whatever t is.
    //
    // A fixed miss cannot do that. "194.35" is one percent off, which
    // is a realistic honest miss at the weather band and a bad quote at
    // the price band, where it earns 0.038. Every attack in this file
    // is measured against this bar, so the bar has to follow the band
    // or the comparisons stop meaning anything.
    let truth = 192.43_f64;
    let miss = truth * (1.0 + eval_script::score::TOLERANCE / 3.0);
    score("192.43", &format!("{miss}"))
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
    //
    // The rule being tested is the anti-spray divisor: the hedge earns
    // the best of its candidates divided by how many it offered. Both
    // 34 and 36 are one unit out from 35, so the best candidate scores
    // exactly what the committed answer "34" scores, and the hedge
    // pays half of it. Asserting that identity states the rule and
    // holds in every band.
    //
    // The bar used to be a flat `range < 0.55`. That is a claim about
    // the weather band wearing no label: the hedge earns 0.0024 at the
    // price band and 0.4825 at the onchain band, so the same literal
    // meant "beaten by 230x" in one band and "beaten by 1.14x" in
    // another, and an onchain tolerance of 0.2 would have failed a
    // scorer that was working correctly.
    let range = score("35", "34 to 36");
    let committed = score("35", "34");
    let exact = score("35", "35");

    assert_eq!(exact, 1.0);
    assert!(
        (range - committed / 2.0).abs() < 1e-12,
        "a hedge of two candidates earned {range}, want half of the committed {committed}"
    );
    assert!(
        range < exact,
        "a hedged range earned {range}, which is not below the committed {exact}"
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
    assert_eq!(echoed, 0.0, "echoing a junk question earned {echoed}");
}

#[test]
fn padding_an_echo_of_the_question_never_escapes_the_check() {
    // THIS IS WHY THE CHECK READS RECALL AND NOT THE OVERLAP SCORE.
    //
    // A threshold on Jaccard falls when the answer grows, so the
    // attacker escaped it by growing the answer: this junk question
    // holds `207`, and an echo with ONE word appended parsed as a
    // number, took the numeric branch, and earned 0.135687 against a
    // ground truth of 192.43. Recall of the question's tokens divides
    // by the QUESTION's size, which the miner does not choose, so no
    // amount of padding moves it.
    let question = "[direct] 207 -> /price";
    let truth = "192.43";

    let mut suffixed = String::from(question);
    for index in 0..10 {
        suffixed.push_str(" filler");
        suffixed.push_str(&index.to_string());
        let earned = score_answer(question, truth, &suffixed);
        assert_eq!(
            earned,
            0.0,
            "an echo with {} words appended earned {earned}",
            index + 1
        );
    }

    // Prefixed, and interleaved through the middle of the question.
    for padded in [
        "zz [direct] 207 -> /price",
        "zz [direct] 207 -> /price yy",
        "[direct] zz 207 yy -> /price",
        "aa bb cc [direct] 207 dd -> ee /price ff gg",
    ] {
        let earned = score_answer(question, truth, padded);
        assert_eq!(earned, 0.0, "the padded echo {padded:?} earned {earned}");
    }
}

#[test]
fn an_honest_answer_that_repeats_the_question_is_not_an_echo() {
    // The check must fire on an answer that gives back the question and
    // NOTHING the truth holds. An answer that repeats the question and
    // then answers it carries the payload, and the payload is a token
    // the truth holds, so the second half of the rule is false.
    let question = "temperature in tokyo";
    let earned = score_answer(question, "34.7 C", "the temperature in tokyo is 34.7 C");
    assert!(
        earned > 0.9,
        "an honest answer that restates the question earned {earned}"
    );

    // The same shape on the text branch.
    let question = "sentiment of the review";
    let earned = score_answer(
        question,
        "positive",
        "the sentiment of the review is positive",
    );
    assert!(
        earned > 0.0,
        "an honest text answer that restates the question earned {earned}"
    );
}

#[test]
fn a_question_that_is_its_own_answer_still_scores() {
    // When the truth IS the question, echoing is correct. The
    // exact-match short circuit runs AFTER this check, so without the
    // guard the right answer would be zeroed before it got there.
    assert_eq!(score_answer("34.7 C", "34.7 C", "34.7 C"), 1.0);
    assert_eq!(score_answer("192.43", "192.43", "192.43"), 1.0);
    // Same words, one doubled space, so byte equality does not save it.
    assert_eq!(score_answer("34.7 C", "34.7 C", "34.7  C"), 1.0);
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

/// The `label` band relaxes dispatch rule 6 on purpose, so the farm
/// this test names is priced differently there. See
/// `the_label_band_pays_for_the_scaffolding_it_admits` in
/// `tests/label_band.rs` for what that band does instead.
#[cfg(not(feature = "label"))]
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

/// The `label` band relaxes dispatch rule 6 on purpose, so the farm
/// this test names is priced differently there. See
/// `the_label_band_pays_for_the_scaffolding_it_admits` in
/// `tests/label_band.rs` for what that band does instead.
#[cfg(not(feature = "label"))]
#[test]
fn repeating_the_unit_many_times_earns_nothing() {
    let earned = score("192.43 USD", "USD USD USD USD USD USD USD USD");
    assert_eq!(earned, 0.0, "the repeated unit earned {earned}");
}

/// The `label` band relaxes dispatch rule 6 on purpose, so the farm
/// this test names is priced differently there. See
/// `the_label_band_pays_for_the_scaffolding_it_admits` in
/// `tests/label_band.rs` for what that band does instead.
#[cfg(not(feature = "label"))]
#[test]
fn giving_back_the_prose_words_without_the_number_earns_nothing() {
    // This is the leak the cross-branch probe found. The answer repeats the
    // scaffolding of the ground truth and omits the value. It scored
    // 0.667 before the fix, while an honest miner 10 percent out
    // scored 0.0826.
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
    // The miss is half of the band's tolerance, so the score is about
    // 0.8 in every band. See `honest_but_imperfect` for why a fixed
    // miss cannot serve every band.
    let miss = 28.9_f64 * (1.0 - eval_script::score::TOLERANCE / 2.0);
    let near = format!("{miss}");
    let bare = score("28.9", &near);
    let prose = score(PROSE_TRUTH, &near);
    let json = score(JSON_TRUTH, &near);
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
        // A tenth of what honest work earns. An absolute bar cannot
        // serve every band: at the onchain band, where the tolerance is
        // 0.15, every wrong number earns more, and "8" against 28.9
        // earns 0.041. The invariant is that farming an incidental
        // number is not worth doing beside answering.
        assert!(
            json < honest_but_imperfect() / 10.0,
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
/// does, must still score. The admission test is `parse_value` on the
/// WHOLE string, so a quoted string that IS one clean value keeps its
/// number while a quoted timestamp does not.
#[test]
fn a_quoted_value_in_a_json_truth_still_scores() {
    for quoted in [
        "{\"temperature_2m\":\"28.9 C\"}",
        "{\"temperature_2m\":\"28.9\"}",
        // parse_value handles the surrounding whitespace and the unit
        // suffix itself, so these need no special case here.
        "{\"temperature_2m\":\" 28.9 \"}",
        "{\"temperature_2m\":\"28.9C\"}",
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
}

/// This test states the price of admitting only a string that IS a
/// value.
///
/// A string that CONTAINS a number is text, and its number is not a
/// match target. That closes the farm below, and it costs the case the
/// old rule was written for: a value wrapped in a sentence.
///
/// The two are the same shape -- a number, then words -- so no test on
/// the string's syntax separates them. The farm is the one that had to
/// go.
#[test]
fn a_number_inside_a_quoted_phrase_is_not_a_match_target() {
    // THE FARM. Every one of these paid a WRONG answer 1.0000 while an
    // honest miner 10 percent out earned 0.0831.
    for (truth, farm) in [
        ("{\"status\":\"HTTP 200\",\"temperature_2m\":28.1}", "200"),
        (
            "{\"summary\":\"3 alerts active\",\"temperature_2m\":28.1}",
            "3",
        ),
        ("{\"note\":\"revision 4\",\"temperature_2m\":28.1}", "4"),
        ("{\"station\":\"KJFK 12\",\"temperature_2m\":28.1}", "12"),
        ("{\"city\":\"Paris 2026\",\"temperature_2m\":28.1}", "2026"),
        ("{\"window\":\"6 hours\",\"temperature_2m\":28.1}", "6"),
    ] {
        let paid = score(truth, farm);
        assert!(
            paid < 0.0831,
            "the farm answer {farm:?} earned {paid} against {truth:?}, \
             at or above the honest bar of 0.0831"
        );
        // The real value still scores in full on the same truth.
        assert_eq!(
            score(truth, "28.1"),
            1.0,
            "the correct answer lost its score against {truth:?}"
        );
    }

    // THE PRICE. The only quantity sits inside a sentence in a quoted
    // string, so this truth now carries no quantity at all and falls to
    // the text branch. The correct answer scored 1.0 before this rule.
    // The `label` band reaches the same branch by its own dispatch
    // rule, so it reads the same here.
    let wrapped = "{\"summary\":\"28.9 C in Paris\"}";
    let correct = score(wrapped, "28.9");
    assert!(
        correct < 1.0,
        "this assertion records a COST, not a gain: {correct}"
    );
    // The scaffolding must not be paid more than the real answer is.
    assert!(
        score(wrapped, "summary") <= correct,
        "the scaffolding out-earned the value"
    );
}
