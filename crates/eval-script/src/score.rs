//! This module holds the scoring rules.
//!
//! The module reads a ground truth text and a miner answer text, and
//! gives a score from 0.0 to 1.0. A high score is good.
//!
//! ## The dispatch order
//!
//! 1. A blank miner answer scores exactly 0.0.
//! 2. Both sides parse as a value, and the units share a family: score
//!    the relative error through the tolerance curve.
//! 3. Both sides parse as a value, and the units are in different
//!    families: score 0.0. A temperature is not near a price.
//! 4. Both sides parse, and one side states a unit while the other
//!    does not: assume the same unit, and charge nothing. The two
//!    sides come from two different pipelines, so a format difference
//!    is not the miner's doing.
//! 5. The ground truth CONTAINS a quantity, and the answer contains
//!    one too: compare the quantities, and charge for every extra
//!    number the answer gave.
//! 6. The ground truth contains a quantity and the answer does not:
//!    score 0.0. The answer did not supply what was asked for.
//! 7. Neither side holds a quantity: compare the texts.
//!
//! ## Why the curve uses arithmetic only
//!
//! Every step of the score runs with `+`, `-`, `*` and `/`. IEEE-754
//! defines each of those as a single correctly rounded operation, so
//! every host gives the same bits. This module calls no `ln`, no
//! `exp`, and no `powf`. Those come from a host maths library, and two
//! hosts can return different last bits for the same input. A one bit
//! difference in a score is a consensus failure, and the validator
//! that differs gets slashed.

use crate::text::{negation_disagrees, overlap_score, tokenize};
use crate::value::{
    parse_value, scan_truth_values, scan_values, Family, ParsedValue, Unit, MAX_SCANNED_VALUES,
};

// Exactly one band feature must be on. Two bands would give two
// `TOLERANCE` definitions and the error would be a confusing duplicate
// symbol, so the build stops here with the reason instead.
#[cfg(any(
    all(feature = "weather", feature = "price"),
    all(feature = "weather", feature = "onchain"),
    all(feature = "price", feature = "onchain"),
))]
compile_error!(
    "eval-script: exactly one tolerance band may be enabled. Cargo keeps the \
     default `weather` feature unless you pass --no-default-features, so build \
     a variant with: --no-default-features --features price"
);

#[cfg(not(any(feature = "weather", feature = "price", feature = "onchain")))]
compile_error!(
    "eval-script: no tolerance band is enabled. Pass --features weather, price, \
     or onchain."
);

/// The relative error that scores one half.
///
/// # THIS IS THE VARIANT POINT
///
/// This constant is the ONE thing a per-intent variant of this script
/// changes. Script registration is per-intent, so a different intent
/// runs a different registered binary. A variant therefore needs no
/// configuration system, no extra input field, and no change to the
/// ABI. The band is a cargo feature, so the value is folded into the
/// curve at compile time and the shipped module holds no branch on it.
///
/// The curve is `score = t^2 / (t^2 + e^2)`, where `e` is the relative
/// error and `t` is this constant. At `e == t` the score is exactly
/// 0.5.
///
/// # The three bands
///
/// | band | `t` | intents |
/// | --- | --- | --- |
/// | `weather` | 0.03 | `WEATHER_CHECK`, `WEATHER_FORECAST` |
/// | `price` | 0.002 | `CRYPTO_PRICE`, `STOCK_PRICE`, `CURRENCY_EXCHANGE`, `FINANCIAL_DATA` |
/// | `onchain` | 0.15 | `GAS_PRICE`, `TVL_LOOKUP` |
///
/// ## `weather = 0.03` is MEASURED
///
/// This is the only band with evidence behind it. It is calibrated
/// against the 6,169-row weather corpus in `docs/EVALUATION.md`. A 1
/// degree miss on a 30 degree day is a 3.3 percent error, which is the
/// boundary between a useful forecast and a poor one. At `t = 0.03` a
/// 1 percent error scores 0.900, a 3 percent error scores 0.500, and a
/// 50 percent error scores 0.0036. The corpus median absolute error
/// across the three live miners is 0.95 to 1.40 C, so the working part
/// of the curve sits where the real errors are.
///
/// ## `price = 0.002` is REASONED, NOT MEASURED
///
/// There is no price corpus in this repository and no measurement
/// behind this number. The reasoning: a quote 3 percent away from the
/// market is not a near miss, it is a bad quote, and at `t = 0.03` it
/// would still score 0.500. `0.002` puts the half-score point at 0.2
/// percent, so a 1 percent error scores 0.038 and a 3 percent error
/// scores 0.0044. That matches the spread a taker actually pays on a
/// liquid pair. It needs the same corpus work the weather band had
/// before anyone should trust the exact figure.
///
/// ## `onchain = 0.15` is REASONED, NOT MEASURED
///
/// Also unmeasured. Gas moves fast and a base fee can double inside a
/// block, so a forecast within 15 percent is useful rather than wrong.
/// At `t = 0.15` a 10 percent error scores 0.692 and a 50 percent error
/// scores 0.083. A TVL lookup is grouped here for the same reason:
/// the quantity is large, it moves with price, and agreement to the
/// percent is not meaningful. Treat the figure as a starting point.
///
/// ## A note the price band does not fix
///
/// A PERCENT or PROBABILITY intent would also want `ABSOLUTE_FLOOR`
/// raised, because those values are small and a relative error on a
/// 0.02 probability is not meaningful. No band here does that, so no
/// band here suits a probability intent.
#[cfg(feature = "weather")]
pub const TOLERANCE: f64 = 0.03;

/// See the `weather` definition above for the full band table. This
/// value is REASONED, NOT MEASURED: no price corpus exists here.
#[cfg(all(feature = "price", not(feature = "weather")))]
pub const TOLERANCE: f64 = 0.002;

/// See the `weather` definition above for the full band table. This
/// value is REASONED, NOT MEASURED: no gas corpus exists here.
#[cfg(all(feature = "onchain", not(feature = "weather"), not(feature = "price")))]
pub const TOLERANCE: f64 = 0.15;

/// The name of the tolerance band this build carries.
///
/// This is a build-time label for the verification script and the
/// report. It is not read by `rank_answer` and costs the module
/// nothing at runtime.
#[cfg(feature = "weather")]
pub const BAND: &str = "weather";
/// See the `weather` definition above.
#[cfg(all(feature = "price", not(feature = "weather")))]
pub const BAND: &str = "price";
/// See the `weather` definition above.
#[cfg(all(feature = "onchain", not(feature = "weather"), not(feature = "price")))]
pub const BAND: &str = "onchain";

/// The smallest divisor for the relative error.
///
/// The relative error divides by the size of the ground truth. A
/// ground truth of 0.0 would divide by zero, and a tiny ground truth
/// would make a small absolute error look enormous. So the divisor is
/// the larger of the ground truth size and this floor.
///
/// The value 1.0 is a judgement call. It suits a temperature, a price,
/// and a percent, which are the quantities in the corpus. It is too
/// large for a quantity whose normal size is far below 1.0, and for
/// such a quantity this floor makes the score too kind. A different
/// intent may need a different floor.
pub const ABSOLUTE_FLOOR: f64 = 1.0;

/// This function scores one ground truth text against one answer text.
///
/// The function always gives a value from 0.0 to 1.0. The caller
/// clamps the result one time, at the ABI boundary.
///
/// `question` is the question text. The function uses it only to
/// defend against an answer that repeats the question. Real traffic
/// carries a question such as "[direct] 207 -> /price", which holds no
/// useful text, so the function never requires the question to hold
/// anything.
pub fn score_answer(question: &str, ground_truth: &str, answer: &str) -> f64 {
    if answer.trim().is_empty() {
        return 0.0;
    }

    // The copied-question check runs BEFORE the dispatch, for every
    // branch.
    //
    // An earlier version ran this check inside the text branch only.
    // That left a hole: a junk question such as "[direct] 207 -> /price"
    // holds the number 207, so an answer that echoed it went to the
    // NUMERIC branch instead, and 207 against a ground truth of 192.43
    // earned 0.1357. The miner gained score for repeating the question
    // back, which is the exact thing the check exists to stop.
    if answer_repeats_question(question, ground_truth, answer) {
        return 0.0;
    }

    // An answer that IS the ground truth is correct, whatever shape the
    // ground truth has.
    //
    // Without this, a ground truth holding several numbers scored its
    // own text below 1.0: the anti-spray divisor in `score_quantities`
    // counts the numbers in the ANSWER, and an answer that repeats a
    // multi-number truth carries all of them. A JSON truth scored 0.500
    // against itself and "CVE-2021-44228 affects log4j" scored 0.333.
    // A registration check that compares a self-match against a
    // cross-match can fail on that.
    //
    // The anti-spray rule is unweakened. It exists to charge a miner
    // that lists numbers it was not asked for, and an answer equal to
    // the ground truth listed nothing extra. No farm reaches this line,
    // because a farm answer differs from the ground truth by
    // construction.
    if ground_truth.trim() == answer.trim() {
        return 1.0;
    }

    // Both sides are one clean value: compare the quantities, with the
    // full unit rules.
    if let (Some(truth), Some(reply)) = (parse_value(ground_truth), parse_value(answer)) {
        return score_two_values(truth, reply);
    }

    // Past that point, at least one side is not a clean single value.
    // The question that decides the branch is NOT "does this text parse
    // as one value" but "does this text CONTAIN a quantity".
    //
    // An earlier version asked the first question. It failed on two
    // real ground-truth renderings from the corpus:
    //
    // - A prose truth, "The temperature was 28.9 C.", did not parse as
    //   one value, so it fell to token overlap. An answer of "the
    //   temperature was C" then scored 0.667 by giving back the
    //   scaffolding words and no number, while an honest miner 10
    //   percent out scored 0.081. The farm paid 8 times better than
    //   real work.
    // - A JSON truth, {"temperature_2m":28.9,...}, holds no whitespace,
    //   so a whitespace split found no number at all and the CORRECT
    //   answer "28.9" scored 0.000.
    //
    // The rule below fixes both. `scan_truth_values` finds a quantity
    // anywhere inside a text, so prose and JSON both yield 28.9.
    //
    // The truth side uses `scan_truth_values`, not `scan_values`. The
    // two differ in one rule: inside a JSON shaped truth, a number that
    // sits in a quoted string is text, not a quantity. Without that
    // rule the timestamp in {"temperature_2m":28.9,"time":"2026-...T12:00"}
    // gave the year, the month, the hour and the minute as free match
    // targets, and an answer of `2026` scored 1.0. See that function
    // for the full reason.
    let mut truth_values = [ZERO_VALUE; MAX_SCANNED_VALUES];
    let truth_count = scan_truth_values(ground_truth, &mut truth_values);

    if truth_count > 0 {
        // The ground truth carries a quantity, so the quantity IS the
        // answer. An answer that supplies no quantity has not answered.
        let mut answer_values = [ZERO_VALUE; MAX_SCANNED_VALUES];
        let answer_count = scan_values(answer, &mut answer_values);
        if answer_count == 0 {
            // No partial credit for restating the words around the
            // number. This is the same treatment a blank answer gets,
            // for the same reason: the miner did not answer.
            return 0.0;
        }
        return score_quantities(&truth_values[..truth_count], &answer_values[..answer_count]);
    }

    // The ground truth carries no quantity, so this is a text answer.
    score_two_texts(ground_truth, answer)
}

/// A placeholder value used to fill a scan buffer before it is written.
const ZERO_VALUE: ParsedValue = ParsedValue {
    number: 0.0,
    unit: Unit::None,
};

/// This function scores a set of answer quantities against a set of
/// ground-truth quantities.
///
/// The function keeps the BEST match over every pair, then divides by
/// the number of DISTINCT quantities the answer gave.
///
/// The divisor is the anti-spray rule. A miner that lists ten numbers
/// and hopes one lands keeps one tenth of the score of a miner that
/// commits to one number. The divisor counts the ANSWER's numbers, not
/// the ground truth's, because the miner controls only its own answer.
///
/// A LIMIT worth stating: when the ground truth holds several numbers,
/// as a JSON rendering with a date does, this function cannot know
/// which one is the wanted value. It takes the best match, so an
/// answer that gives back a date part from the ground truth scores
/// well. The alternative, dividing by the ground-truth count too,
/// would punish an honest miner for the way the ground truth happens
/// to be rendered, which the miner does not control. The evaluation
/// report measures this case rather than hiding it.
fn score_quantities(truth_values: &[ParsedValue], answer_values: &[ParsedValue]) -> f64 {
    let mut best = 0.0f64;
    for truth in truth_values {
        for reply in answer_values {
            let pair = score_two_values(*truth, *reply);
            if pair > best {
                best = pair;
            }
        }
    }

    let mut distinct = 0usize;
    let mut seen = [0.0f64; MAX_SCANNED_VALUES];
    for reply in answer_values {
        let stored = distinct.min(seen.len());
        if seen[..stored].contains(&reply.number) {
            continue;
        }
        if distinct < seen.len() {
            seen[distinct] = reply.number;
        }
        distinct += 1;
    }
    if distinct == 0 {
        return 0.0;
    }

    // The counts are small, so this conversion is exact.
    best / (distinct as f64)
}

/// This function scores two parsed values against each other.
///
/// The function handles the three unit cases: the same family, two
/// different families, and one side without a unit.
fn score_two_values(truth: ParsedValue, reply: ParsedValue) -> f64 {
    let truth_family = truth.family();
    let reply_family = reply.family();

    // Case C: one side states a unit and the other does not. Read the
    // bare number in the unit of the other side, and charge nothing
    // for the missing unit.
    //
    // There is no penalty in either direction. The ground truth and
    // the miner answer arrive through two different pipelines, and
    // the protocol team keeps both pipelines undisclosed. The miner
    // answer is a single value that the protocol standardises before
    // `rank_answer` ever sees it. So the miner does not choose
    // whether its unit survives that step.
    //
    // A penalty here would therefore charge every miner the same
    // amount for a detail outside its control. It would measure the
    // shape of the pipeline, not the quality of the answer, and the
    // score must measure only the answer.
    //
    // The unit still matters when BOTH sides state one. A spoofed
    // unit and an incompatible unit both still score 0.0; see the two
    // branches below.
    // A BARE GROUND TRUTH is read as already being in the BASE unit of
    // the answer's family, not in the answer's own unit.
    //
    // The two sides are not symmetric. The ground truth comes from the
    // protocol's own pipeline, so it is canonical. A miner answer does
    // not have to be. So a bare ground truth of "28.9" against an
    // answer of "302.05 K" means 28.9 CELSIUS, the base unit, and the
    // two agree exactly.
    //
    // An earlier version read the bare side in the ANSWER's unit. That
    // made the same pair mean 28.9 K against 302.05 K, which scored
    // 0.0007. The identical answer scored 1.000 against the prose
    // rendering of the same truth, because prose carries the "C". A
    // score that changes with the rendering of the truth is a defect:
    // the miner does not choose that rendering.
    if truth_family == Family::Dimensionless && reply_family != Family::Dimensionless {
        return relative_score(truth.number, reply.to_base());
    }
    // A BARE ANSWER is read in the GROUND TRUTH's unit, which is the
    // opposite rule, for the same reason. The truth states the unit it
    // wants, and an answer that repeats the truth's own magnitude is
    // most likely quoting that same unit.

    if reply_family == Family::Dimensionless && truth_family != Family::Dimensionless {
        let assumed = ParsedValue {
            number: reply.number,
            unit: truth.unit,
        };
        return relative_score(truth.to_base(), assumed.to_base());
    }

    // Case B: two different quantity families do not compare.
    if truth_family != reply_family {
        return 0.0;
    }

    // Case A: the same family. Convert both to the base unit.
    relative_score(truth.to_base(), reply.to_base())
}

/// This function compares two texts that hold no clean value.
///
/// The function scores the token overlap, then applies the negation
/// rule. The caller runs the copied-question rule before it dispatches
/// to this function, so every branch gets that check, not only this
/// one.
fn score_two_texts(ground_truth: &str, answer: &str) -> f64 {
    let truth_tokens = tokenize(ground_truth);
    let answer_tokens = tokenize(answer);

    // Case F: the two sides disagree about negation. The answer states
    // the opposite of the ground truth, so it is wrong, not partly
    // right.
    if negation_disagrees(&truth_tokens, &answer_tokens) {
        return 0.0;
    }

    overlap_score(&truth_tokens, &answer_tokens)
}

/// This function tells if the answer is a copy of the question.
///
/// The function returns true when the answer tokens match the question
/// tokens and the ground truth tokens do not. A question that shares
/// its wording with the ground truth never triggers this rule, because
/// then the answer may be right for an honest reason.
///
/// The caller runs this check before the dispatch, so it covers the
/// numeric branch as well as the text branch. A junk question often
/// holds a number, and without this check an echo of the question
/// could earn a numeric score.
fn answer_repeats_question(question: &str, ground_truth: &str, answer: &str) -> bool {
    let question_tokens = tokenize(question);
    let truth_tokens = tokenize(ground_truth);
    let answer_tokens = tokenize(answer);
    if question_tokens.is_empty() || answer_tokens.is_empty() {
        return false;
    }
    let answer_matches_question = overlap_score(&question_tokens, &answer_tokens);
    let truth_matches_question = overlap_score(&question_tokens, &truth_tokens);
    answer_matches_question > 0.99 && truth_matches_question < 0.99
}

/// This function turns a relative error into a score.
///
/// The curve is `t^2 / (t^2 + e^2)`, with `t` set by `TOLERANCE`.
///
/// The curve has the properties a score needs:
/// - It gives exactly 1.0 for an exact match, because `e` is 0.
/// - It falls without a step, so it separates every pair of answers.
///   A threshold rule gives the same 0.0 to an answer that is one cent
///   out and to an answer that is a million out. This curve gives
///   0.99942 and 0.0000000003 for those two.
/// - It never goes below 0.0 and never goes above 1.0.
/// - It uses arithmetic only, so every host agrees on the bits.
///
/// The divisor for the relative error is the size of the GROUND TRUTH,
/// never the size of the answer. A divisor that used the answer would
/// let a miner shrink its own error by sending a huge number, because
/// a huge number would make the divisor huge too.
pub fn relative_score(truth: f64, answer: f64) -> f64 {
    if !truth.is_finite() || !answer.is_finite() {
        return 0.0;
    }

    let difference = truth - answer;
    if difference == 0.0 {
        return 1.0;
    }

    let magnitude = if truth < 0.0 { -truth } else { truth };
    let divisor = if magnitude > ABSOLUTE_FLOOR {
        magnitude
    } else {
        ABSOLUTE_FLOOR
    };

    let absolute_difference = if difference < 0.0 {
        -difference
    } else {
        difference
    };
    let relative_error = absolute_difference / divisor;

    let tolerance_squared = TOLERANCE * TOLERANCE;
    let error_squared = relative_error * relative_error;
    let denominator = tolerance_squared + error_squared;
    if !denominator.is_finite() || denominator <= 0.0 {
        // A relative error large enough to overflow scores 0.0.
        return 0.0;
    }
    let result = tolerance_squared / denominator;
    if result.is_finite() {
        result
    } else {
        0.0
    }
}
