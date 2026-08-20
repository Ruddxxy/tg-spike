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

use crate::text::{
    intersection_size, negation_disagrees, overlap_score, substitution_score, tokenize,
};
use crate::value::{
    parse_value, scan_truth_values, scan_values, Family, ParsedValue, Unit, MAX_SCANNED_VALUES,
};

// Exactly one band feature must be on. Two bands would give two
// `TOLERANCE` definitions and the error would be a confusing duplicate
// symbol, so the build stops here with the reason instead.
#[cfg(any(
    all(feature = "weather", feature = "price"),
    all(feature = "weather", feature = "onchain"),
    all(feature = "weather", feature = "label"),
    all(feature = "weather", feature = "metadata"),
    all(feature = "price", feature = "onchain"),
    all(feature = "price", feature = "label"),
    all(feature = "price", feature = "metadata"),
    all(feature = "onchain", feature = "label"),
    all(feature = "onchain", feature = "metadata"),
    all(feature = "label", feature = "metadata"),
))]
compile_error!(
    "eval-script: exactly one band may be enabled. Cargo keeps the \
     default `weather` feature unless you pass --no-default-features, so build \
     a variant with: --no-default-features --features price"
);

#[cfg(not(any(
    feature = "weather",
    feature = "price",
    feature = "onchain",
    feature = "label",
    feature = "metadata"
)))]
compile_error!(
    "eval-script: no band is enabled. Pass --features weather, price, onchain, \
     label or metadata."
);

/// The relative error that scores one half.
///
/// # THIS IS THE MAIN VARIANT POINT
///
/// This constant is what three of the four bands change, and nothing
/// else. Script registration is per-intent, so a different intent runs
/// a different registered binary. A variant therefore needs no
/// configuration system, no extra input field, and no change to the
/// ABI. The band is a cargo feature, so the value is folded into the
/// curve at compile time and the shipped module holds no branch on it.
///
/// The fourth band, `label`, is the exception and it does NOT change
/// this constant. It changes one dispatch rule instead: a truth that
/// carries a number no longer makes a number mandatory. See
/// `answer_without_a_quantity` below for that rule and for what it
/// costs. Read "the variant point" as "the tolerance variant point".
///
/// The curve is `score = t^2 / (t^2 + e^2)`, where `e` is the relative
/// error and `t` is this constant. At `e == t` the score is exactly
/// 0.5.
///
/// # The four bands
///
/// | band | `t` | changes | intents |
/// | --- | --- | --- | --- |
/// | `weather` | 0.03 | the curve | `WEATHER_CHECK`, `WEATHER_FORECAST` |
/// | `price` | 0.002 | the curve | `CRYPTO_PRICE`, `STOCK_PRICE`, `CURRENCY_EXCHANGE`, `FINANCIAL_DATA` |
/// | `onchain` | 0.15 | the curve | `GAS_PRICE`, `TVL_LOOKUP` |
/// | `label` | 0.03 | dispatch rule 6 | `URL_SCAN`, `SSL_VERIFICATION`, `CVE_LOOKUP`, `SENTIMENT_ANALYSIS`, `TEXT_CLASSIFICATION`, `CONTENT_MODERATION`, `FACT_CHECK`, `LANGUAGE_TRANSLATION` |
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

/// The `label` band. This value is NOT MEASURED and it is not what the
/// band is for.
///
/// A label intent asks for a verdict, a grade, a severity or a
/// language, not for a quantity. The band exists for the DISPATCH rule
/// below, not for this constant: see `answer_without_a_quantity`. The
/// tolerance still has to hold a value, because a label truth can carry
/// an incidental number and an answer can quote it, so the number is
/// kept at the weather figure. Nothing measured that choice for a label
/// intent.
#[cfg(all(
    feature = "label",
    not(feature = "weather"),
    not(feature = "price"),
    not(feature = "onchain")
))]
pub const TOLERANCE: f64 = 0.03;

/// The `metadata` band. Held at the weather figure for the same reason
/// the `label` band is, and measured for a label intent no more than
/// that one was. The band exists for `RULE6_ATTENUATION` below, not for
/// this constant.
#[cfg(all(
    feature = "metadata",
    not(feature = "weather"),
    not(feature = "price"),
    not(feature = "onchain"),
    not(feature = "label")
))]
pub const TOLERANCE: f64 = 0.03;

/// What an answer that carries NO quantity keeps of its text score,
/// when the ground truth carries one.
///
/// # THIS IS THE SECOND VARIANT POINT
///
/// `TOLERANCE` above is what three bands change. This constant is what
/// the other two change, and it is the whole of dispatch rule 6:
///
/// ```text
/// answer_without_a_quantity = RULE6_ATTENUATION * score_two_texts(truth, answer)
/// ```
///
/// | band | `alpha` | an answer with no quantity |
/// | --- | --- | --- |
/// | `weather`, `price`, `onchain` | 0.0 | earns EXACTLY nothing |
/// | `metadata` | 0.03595 | earns a fraction, always under the honest bar |
/// | `label` | 1.0 | earns the full text score |
///
/// ## Why a fraction, and not a narrower trigger
///
/// The obvious repair is to fire rule 6 only when the truth's quantity
/// is the thing the question asked for. Seven candidate triggers were
/// measured for that and every one was rejected. The measure that came
/// closest puts a farm answer and a correct label answer at the SAME
/// value of 2/7, so no threshold on it separates them at any setting.
/// `cargo run -p corpus-eval --example rule6_probe -- --report`
/// reproduces that, and the reason is that the discriminator is the
/// INTENT, which `rank_answer` is never given.
///
/// A fraction does not have to decide. The farm and the correct label
/// answer both pay it, and the two have different requirements: the
/// farm must stay under an absolute bar, and the label answer must only
/// stay above a wrong answer sitting at 0.0. A constant satisfies both
/// where a threshold cannot.
///
/// ## What sets each end of the range
///
/// The open window for this band is `(0.014348, 0.090075)` and
/// `0.03595` is its geometric mean, so it sits as far from both walls
/// as the window allows.
///
/// - The TOP is set by the worst farm. `alpha` times the largest score
///   the text branch ever pays a no-quantity answer must stay under the
///   honest bar, which is 0.082569: what a miner earns when it gives a
///   real number and is 10 percent out at `t = 0.03`.
/// - The BOTTOM is set by the tightest label pair. On the FACT_CHECK
///   row whose truth is "Partly true. The programme reduced
///   transmission by 40%.", the correct "partly true" carries no
///   quantity and is attenuated, while the wrong "60%" carries one and
///   reaches the numeric branch untouched at 0.0036. So
///   `alpha * 0.25 > 0.0036`.
///
/// ## The ceiling this rests on
///
/// The largest score measured for a no-quantity answer is 0.9167, on a
/// 23-token truth whose answer is the truth with the number taken out.
/// A longer truth makes the one missing token a smaller share, so that
/// figure CLIMBS towards 1.0 and is a floor on the true ceiling rather
/// than the ceiling.
///
/// The window does not depend on having found the worst case. Taking
/// the ceiling as 1.0 outright, the top wall becomes 0.082569 and the
/// window is still open. `0.03595` clears it by a factor of 2.3.
///
/// ## What this costs
///
/// The other three bands keep an EXACT-ZERO guarantee: an answer with
/// no quantity earns nothing, structurally, and no constant has to be
/// right for that to hold. This band trades that for a quantitative
/// guarantee: an answer with no quantity earns less than an honest
/// miner, PROVIDED `alpha` is correct. That is a weaker claim and it is
/// the price of the six cases. Register this band only on an intent
/// whose wanted answer is a word.
#[cfg(not(any(feature = "label", feature = "metadata")))]
pub const RULE6_ATTENUATION: f64 = 0.0;

/// See the definition above for the full table and the derivation.
#[cfg(feature = "label")]
pub const RULE6_ATTENUATION: f64 = 1.0;

/// See the definition above for the full table and the derivation.
#[cfg(all(feature = "metadata", not(feature = "label")))]
pub const RULE6_ATTENUATION: f64 = 0.03595;

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
/// See the `weather` definition above.
#[cfg(all(
    feature = "label",
    not(feature = "weather"),
    not(feature = "price"),
    not(feature = "onchain")
))]
pub const BAND: &str = "label";
/// See the `weather` definition above.
#[cfg(all(
    feature = "metadata",
    not(feature = "weather"),
    not(feature = "price"),
    not(feature = "onchain"),
    not(feature = "label")
))]
pub const BAND: &str = "metadata";

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
    //   percent out scored 0.0826. The farm paid 8 times better than
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
            return answer_without_a_quantity(ground_truth, answer);
        }
        // The divisor needs the truth's numbers INCLUDING the ones
        // inside quoted strings. See `score_quantities` for why the
        // two scans differ.
        let mut truth_lenient = [ZERO_VALUE; MAX_SCANNED_VALUES];
        let lenient_count = scan_values(ground_truth, &mut truth_lenient);
        return score_quantities(
            &truth_values[..truth_count],
            &truth_lenient[..lenient_count],
            &answer_values[..answer_count],
        );
    }

    // The ground truth carries no quantity, so this is a text answer.
    score_two_texts(ground_truth, answer)
}

/// This function scores an answer that holds no quantity, against a
/// ground truth that holds one. It is dispatch rule 6, and it is the
/// ONE rule the `label` band changes.
///
/// # Every band except `label`: 0.0
///
/// The quantity IS the answer, so an answer without one has not
/// answered. There is no partial credit for restating the words around
/// the number. This is the same treatment a blank answer gets, for the
/// same reason.
///
/// The rule is load bearing. Without it, the truth
/// "The temperature was 28.9 C." paid 0.667 for the answer
/// "the temperature was C", which gives back the scaffolding and no
/// value, while an honest miner 10 percent out earned 0.0826. Never
/// relax it in a band whose answer is a quantity.
#[cfg(not(any(feature = "label", feature = "metadata")))]
fn answer_without_a_quantity(_ground_truth: &str, _answer: &str) -> f64 {
    0.0
}

/// This function scores an answer that holds no quantity, against a
/// ground truth that holds one. See the first definition of this
/// function for the rule the numeric bands use.
///
/// # The `metadata` band: a fraction of the text comparison
///
/// This band serves the same intents the `label` band does, and it
/// answers the same complaint: a truth that carries a confidence beside
/// a verdict, or a CVSS beside a severity, made a number mandatory and
/// killed the correct word.
///
/// It differs in what it pays. `label` pays the full text score, which
/// closes those cases and reopens the scaffolding farm: the truth "The
/// temperature was 28.9 C." pays 0.6667 for "the temperature was C",
/// which holds no temperature, against 0.0826 for an honest miner 10
/// percent out. This band pays `RULE6_ATTENUATION` of that score, which
/// is 0.0330 for the same farm and keeps every measured farm under the
/// bar while every one of the six cases still separates.
///
/// The multiply is a single IEEE-754 operation with one rounding step,
/// so every host gives the same bits. See `RULE6_ATTENUATION` for how
/// the constant was derived and for what the band gives up.
#[cfg(all(feature = "metadata", not(feature = "label")))]
fn answer_without_a_quantity(ground_truth: &str, answer: &str) -> f64 {
    RULE6_ATTENUATION * score_two_texts(ground_truth, answer)
}

/// This function scores an answer that holds no quantity, against a
/// ground truth that holds one. See the other definition of this
/// function for the rule every other band uses.
///
/// # The `label` band: compare the texts
///
/// A label intent asks for a verdict, a grade, a severity, a language
/// or a translation. The wanted answer is a WORD. The ground truth
/// still often carries a number that nobody asked for: a confidence
/// beside the verdict, a CVSS score beside the severity, a protocol
/// version beside the grade, the digits of an identifier.
///
/// In every other band that number makes a number mandatory, and the
/// correct word then scores 0.0. Measured on the 40 question promotion
/// benchmark, six rows scored 0.0 for BOTH the good and the bad answer,
/// so they gave the node nothing to compare. On one of them the truth
/// "Partly true. The programme reduced transmission by 40%." paid
/// 0.0000 for the correct "partly true" and 0.0036 for the wrong "60%".
///
/// This band sends that case to the text comparison instead. The farm
/// the rule above stops does not exist here, because here the words ARE
/// the answer.
///
/// Registration is per intent, so this is a separate registered binary
/// and no numeric band is touched by it.
#[cfg(feature = "label")]
fn answer_without_a_quantity(ground_truth: &str, answer: &str) -> f64 {
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
/// the number of DISTINCT quantities the answer GUESSED.
///
/// The divisor is the anti-spray rule. A miner that lists ten numbers
/// and hopes one lands keeps one tenth of the score of a miner that
/// commits to one number. The divisor counts the ANSWER's numbers, not
/// the ground truth's, because the miner controls only its own answer.
///
/// ## Why a number the ground truth holds is not counted
///
/// When the answer gives back EVERY quantity the truth holds, the
/// divisor counts only the answer numbers that the ground truth does
/// NOT hold. A number the truth already carries is not a guess:
/// the miner is quoting the truth back, and quoting is what a correct
/// answer looks like when the truth is a sentence with an identifier or
/// a date in it.
///
/// Without this rule the score of an answer depended on how many
/// numbers the TRUTH happened to carry, which the miner does not
/// choose. An answer EQUAL to the truth
/// "CVE-2021-44228 has a severity rating of CRITICAL." scored 0.5,
/// because it carried the two numbers of the identifier, and the same
/// held for "INVOICE 2024-001". Both reached 1.0 only through the
/// exact-match short circuit above, which needs BYTE equality: one
/// doubled space took them back to 0.5. A registration check that
/// compares a self match against the 0.75 floor fails on that, and
/// nothing here controls how the node builds the answer it sends.
///
/// The rule costs the anti-spray rule NOTHING. A spray is a list of
/// GUESSES, and a guess that misses is still counted. The exemption
/// needs the answer to hold every number the truth holds AND no number
/// it does not, so a spray never reaches it: the numbers it added are
/// its own. A spray of five numbers with one right pays 0.200, which is
/// what it paid before this rule existed.
///
/// An answer with NO number never reaches this function. The caller
/// returns 0.0 before it, so the farm that returns the words around a
/// value and no value keeps its 0.0.
///
/// A LIMIT worth stating: when the ground truth holds several numbers,
/// as a JSON rendering with a date does, this function cannot know
/// which one is the wanted value. It takes the best match, so an
/// answer that gives back a date part from the ground truth scores
/// well. The alternative, dividing by the ground-truth count too,
/// would punish an honest miner for the way the ground truth happens
/// to be rendered, which the miner does not control. The evaluation
/// report measures this case rather than hiding it.
fn score_quantities(
    truth_values: &[ParsedValue],
    truth_quoted_included: &[ParsedValue],
    answer_values: &[ParsedValue],
) -> f64 {
    let mut best = 0.0f64;
    for truth in truth_values {
        for reply in answer_values {
            let pair = score_two_values(*truth, *reply);
            if pair > best {
                best = pair;
            }
        }
    }

    if answer_values.is_empty() {
        return 0.0;
    }

    // Does the answer give back EVERY quantity the truth asked for?
    //
    // This is the gate on the quoting rule below, and it is what keeps
    // the rule from paying a wrong answer. An answer that holds every
    // target holds the wanted value by construction, whatever else it
    // adds. An answer that holds only SOME of them has guessed, and it
    // pays the full divisor exactly as it did before.
    //
    // The measured case: the truth "INVOICE 2024-001" and the wrong
    // answer "INVOICE 2024-002". Without this gate the wrong invoice
    // number kept the 2024 for free and scored 1.0, level with the
    // right one. With it, the answer misses the 001 target, so it pays
    // for both of its numbers and scores 0.5.
    //
    // A restatement also ADDS NOTHING. Without that second half the
    // rule pays the prose farm in full: against the truth 28.1, the
    // answer "Wind 28.1 kph, temperature 34.9 C." holds every target,
    // so the 28.1 stopped being charged and only the 34.9 was left in
    // the divisor. Best match over divisor 1 is 1.0000, and a wrong
    // answer that names the right number as some OTHER quantity scored
    // a perfect result against an honest bar of 0.0831. Requiring the
    // answer to add no number of its own puts that back to 0.5000,
    // which is what it paid before the quoting rule existed.
    //
    // The two halves say the same thing from each side: the answer
    // holds every number the truth holds, and no number it does not.
    // That is a restatement. Anything else is a guess and pays.
    let answer_holds_every_target = truth_values
        .iter()
        .all(|truth| answer_values.iter().any(|r| r.number == truth.number));
    let answer_adds_no_number = answer_values.iter().all(|reply| {
        truth_quoted_included
            .iter()
            .any(|truth| truth.number == reply.number)
    });
    let answer_restates_the_truth = answer_holds_every_target && answer_adds_no_number;

    let mut guesses = 0usize;
    let mut seen = [0.0f64; MAX_SCANNED_VALUES];
    for reply in answer_values {
        // A number the ground truth itself holds is a quote, not a
        // guess. See the doc comment above.
        //
        // The test uses the LENIENT scan of the truth, the one that
        // keeps the digits inside a quoted string. Those digits are not
        // match TARGETS, and the loop above still cannot score against
        // them, so the JSON number farm stays shut. But an answer that
        // restates a JSON truth carries them, and charging the miner
        // for a digit inside the truth's own key name would bring back
        // the defect this rule removes: the JSON rendering
        // {"temperature_2m":28.9,"wind_speed_10m":11.2} scored its own
        // restatement 0.5, because `temperature_2m` and
        // `wind_speed_10m` gave the answer a 2 and a 10 to be charged
        // for.
        if answer_restates_the_truth
            && truth_quoted_included
                .iter()
                .any(|truth| truth.number == reply.number)
        {
            continue;
        }
        let stored = guesses.min(seen.len());
        if seen[..stored].contains(&reply.number) {
            continue;
        }
        if guesses < seen.len() {
            seen[guesses] = reply.number;
        }
        guesses += 1;
    }

    // An answer that guessed nothing quoted the truth and nothing else,
    // so it pays no divisor.
    let divisor = if guesses == 0 { 1 } else { guesses };

    // The counts are small, so this conversion is exact.
    best / (divisor as f64)
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

    // Case G: the answer gives back the truth's wording and puts
    // something else where the payload was. That is a substitution, not
    // a partial answer, and it is charged as one. See
    // `substitution_score`.
    substitution_score(&truth_tokens, &answer_tokens)
}

/// This function tells if the answer is a copy of the question.
///
/// The rule has two halves, and an answer must meet both:
///
/// 1. The answer gives back EVERY token of the question.
/// 2. Everything the answer adds beyond the question is FOREIGN to the
///    ground truth.
///
/// An answer that does both repeated the question and supplied nothing
/// the truth would recognise. That is an echo whatever else it carries.
///
/// # Why neither half reads a similarity score
///
/// An earlier version asked whether the Jaccard overlap of the question
/// and the answer was above 0.99. That is a threshold on a similarity
/// that FALLS when the answer grows, so the attacker escaped it by
/// growing the answer. Against the real question `[direct] 207 ->
/// /price` and a ground truth of `192.43`, the echo scored 0.0000 and
/// the echo with one word appended scored 0.135687 -- the whole of the
/// defect the check exists to remove, back for one token.
///
/// Half 1 reads RECALL of the question's tokens instead. Recall divides
/// by the QUESTION's size, which the miner does not choose, so padding
/// the answer cannot move it. This is the same reason
/// `substitution_score` charges on recall and not on the overlap score.
///
/// Half 2 is what keeps an honest answer safe. An answer that repeats
/// the question and then answers it carries the payload, and the
/// payload is a token the truth holds, so half 2 is false and the rule
/// does not fire. Only an answer whose additions are all foreign to the
/// truth is an echo. Padding cannot help there either: every filler
/// word an attacker adds is foreign by construction, so it keeps half 2
/// true rather than escaping it.
///
/// # The question that is its own answer
///
/// A question that shares its wording with the ground truth never
/// triggers this rule. When the truth IS the question, echoing is the
/// correct answer, and half 2 would otherwise be vacuously true for an
/// answer that gives back the truth exactly. The exact-match short
/// circuit in `score_answer` runs AFTER this check, so without this
/// guard a correct answer would be scored 0.0 before it reached it.
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

    // The truth is the question, so echoing it is the right answer.
    if overlap_score(&question_tokens, &truth_tokens) > 0.99 {
        return false;
    }

    // Half 1: recall of the question, which the answer's length cannot
    // dilute.
    let answer_holds_all_of_the_question =
        intersection_size(&question_tokens, &answer_tokens) == question_tokens.len();
    if !answer_holds_all_of_the_question {
        return false;
    }

    // Half 2: everything past the question is foreign to the truth.
    answer_tokens
        .tokens()
        .iter()
        .all(|token| question_tokens.contains(token) || !truth_tokens.contains(token))
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
