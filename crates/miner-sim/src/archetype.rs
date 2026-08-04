//! This module makes the miner responses for each archetype.
//!
//! An archetype has a known true quality. This module builds a
//! response that carries that quality. The core idea: the function
//! draws the certainty first, then decides correctness from the
//! certainty. The function never picks correctness first and fakes a
//! confidence number to match it.

use crate::rng::Rng;
use crate::types::{Archetype, Dataset, Item, Response, ResponseKind, ResponseSeed};

/// The five malformed response bodies that `Malformer` sends.
///
/// The archetype rotates through this list one form per malformed
/// item. `"{}"` gives valid JSON with a missing field. The `eval-script`
/// crate treats a missing field as an error, so this module marks that
/// form `Malformed`, not `Abstain`. The true `Abstain` response also
/// uses the text `"{}"`, but this module tags it with a different
/// `ResponseKind`. The two uses of the same text carry different
/// meaning: one is an intended non-answer, the other is a miner bug
/// that this simulator plants on purpose.
const MALFORMED_FORMS: [&str; 5] = [
    "{}",
    "{invalid json",
    "{\"confidence\": \"high\"}",
    "{\"confidence\": 1e400}",
    "",
];

/// This function makes one response for one item.
///
/// The function reads the archetype and picks the matching behaviour.
/// It returns the response that the archetype gives for this item.
pub fn respond(archetype: Archetype, item: &Item, dataset: &Dataset, rng: &mut Rng) -> Response {
    match archetype {
        Archetype::Oracle => oracle_response(item),
        Archetype::NoisyGood => {
            let (reported, correct) = calibrated_answer(0.85, item, rng);
            make_answer_response(reported, correct)
        }
        Archetype::NoisyMediocre => {
            let (reported, correct) = calibrated_answer(0.65, item, rng);
            make_answer_response(reported, correct)
        }
        Archetype::ConstantMajority => constant_majority_response(item, dataset),
        Archetype::Random => random_response(item, rng),
        Archetype::OverconfidentGood => {
            let (reported, correct) = calibrated_answer(0.85, item, rng);
            let sharpened = sharpen_toward_extreme(reported, 2.5);
            make_answer_response(sharpened, correct)
        }
        Archetype::UnderconfidentGood => {
            let (reported, correct) = calibrated_answer(0.85, item, rng);
            let softened = sharpen_toward_extreme(reported, 0.4);
            make_answer_response(softened, correct)
        }
        Archetype::Abstainer => abstainer_response(item, dataset, rng),
        Archetype::Malformer => malformer_response(item, rng),
        Archetype::Contrarian => contrarian_response(item, rng),
        Archetype::BayesCalibratedGood => bayes_calibrated_good_response(item, dataset, rng),
    }
}

/// This function makes the responses of one archetype for a whole data
/// set.
///
/// The function makes one `Rng` from `seed`, then calls `respond` once
/// per item, in item order. The function is pure: the same archetype,
/// data set, and seed always give the same output, byte for byte.
///
/// # Trap: pick a response seed that differs from the data set seed
///
/// Pass a `seed` that is not the same value as `dataset.seed`. The
/// `Rng` in this function and the `Rng` that built the data set run
/// the exact same algorithm. If the two seeds match, and an archetype
/// draws the same count of random values per item as `generate` did,
/// every miner draw for an item lines up with the exact draws that
/// built that same item's label and signal. That line-up makes the
/// miner's confidence a hidden function of its own item's signal, on
/// top of the intended damping in `calibrated_answer`. Measured
/// accuracy then drifts far from the target accuracy — in one check,
/// `NoisyGood` measured 0.751 correct instead of the expected 0.675
/// when the response seed matched the data set seed, on a data set
/// where the signal is uniform. Always pass a response seed that
/// differs from the data set seed.
///
/// # The newtype now enforces this rule
///
/// The `seed` parameter has type [`ResponseSeed`], not `u64`. `Dataset`
/// cannot make a `ResponseSeed`, and there is no `From<u64>` for it. A
/// caller must use [`ResponseSeed::derive`], which applies a fixed mask
/// away from the data set seed, or state the risk out loud with
/// [`ResponseSeed::new_unchecked`]. This closes the trap above at
/// compile time, not just by convention.
#[must_use]
pub fn responses_for(archetype: Archetype, dataset: &Dataset, seed: ResponseSeed) -> Vec<Response> {
    let mut rng = Rng::new(seed.get());
    dataset
        .items
        .iter()
        .map(|item| respond(archetype, item, dataset, &mut rng))
        .collect()
}

/// This function builds the calibrated core of a "good" miner.
///
/// The function takes a target accuracy `a` in the open range 0.5 up
/// to 1.0. It gives back the reported confidence and a flag that says
/// if the answer is correct.
///
/// # The method
///
/// 1. The function damps `a` by the item signal, so a hard item is
///    hard for every miner:
///    `a_eff = 0.5 + (a - 0.5) * item.signal`.
///    A signal of 1.0 keeps the full accuracy `a`. A signal of 0.0
///    drops the accuracy to 0.5, a coin flip.
/// 2. The function draws the certainty `c` before it knows if the
///    answer will be correct. It draws `u` from `next_f64` and sets
///    `c = 0.5 + 0.5 * u.powf(gamma)`, where
///    `gamma = 0.5 / (a_eff - 0.5) - 1.0`.
///
///    This construction gives `c` in the range 0.5 up to 1.0, with
///    `E[c] = a_eff`. Here is the proof. For `u` uniform on 0.0 up to
///    1.0 and `gamma > 0`:
///    `E[u^gamma] = integral from 0 to 1 of u^gamma du`
///    `           = [u^(gamma+1) / (gamma+1)] from 0 to 1`
///    `           = 1 / (gamma + 1)`.
///    So `E[c] = 0.5 + 0.5 * (1 / (gamma + 1))`. Put in the value of
///    `gamma`: `gamma + 1 = 0.5 / (a_eff - 0.5)`, so
///    `1 / (gamma + 1) = (a_eff - 0.5) / 0.5`, and
///    `E[c] = 0.5 + 0.5 * (a_eff - 0.5) / 0.5 = 0.5 + (a_eff - 0.5) = a_eff`.
///    The expected certainty equals the target accuracy, as required.
///
///    Guard: when `a_eff` is at or under `0.5 + 1e-9`, `gamma` would
///    overflow (division by a value near 0). In that case the function
///    sets `c = 0.5` right away and does **not** draw `u`. This choice
///    changes the length of the draw sequence for that one item. The
///    function makes this choice on purpose and keeps it every time,
///    so the same input always skips the same draws.
///
///    A note on `powf`: this module runs inside the simulator, not
///    inside `eval-script`. Only `eval-script` must avoid the std
///    transcendental functions, because that crate must match the
///    Canonical Script's math bit for bit. This module has no such
///    limit, so `powf` is safe and correct to use here.
/// 3. The function decides correctness from the certainty:
///    `correct = rng.bernoulli(c)`. This step always draws one value.
/// 4. The function picks the predicted label from `correct`: the
///    prediction agrees with the true label when `correct` is true,
///    and disagrees otherwise.
/// 5. The function reports the probability of label 1: `c` when the
///    prediction is 1, or `1.0 - c` when the prediction is 0.
///
/// # Calibration holds only at base rate 0.50
///
/// This construction gives a calibrated confidence — `P(label = 1)`
/// given a reported value `r` — only when the data set base rate is
/// 0.50. At a general base rate `B`, Bayes' rule gives:
/// `P(label = 1 | reported = r) = (B * r) / (B * r + (1 - B) * (1 - r))`.
/// With `B = 0.5` this equals `r` exactly. With a skewed base rate,
/// such as `B = 0.9`, it does not equal `r`. This gap is a fact about
/// the data set, not a bug in this function. The calibration test in
/// `tests/calibration.rs` checks the balanced data set for this
/// reason, and checks the skewed data set against the formula above,
/// not against `r`.
fn calibrated_answer(a: f64, item: &Item, rng: &mut Rng) -> (f64, bool) {
    let a_eff = 0.5 + (a - 0.5) * item.signal;
    let c = if a_eff <= 0.5 + 1e-9 {
        0.5
    } else {
        let gamma = 0.5 / (a_eff - 0.5) - 1.0;
        let u = rng.next_f64();
        0.5 + 0.5 * u.powf(gamma)
    };
    let correct = rng.bernoulli(c);
    let predicted: u8 = if correct { item.label } else { 1 - item.label };
    let reported = if predicted == 1 { c } else { 1.0 - c };
    (reported, correct)
}

/// This function pushes a probability toward 0.0 or 1.0, or pulls it
/// toward 0.5, without moving the 0.5 point and without changing which
/// side of 0.5 the value is on.
///
/// The function uses the odds-power transform
/// `r' = r^k / (r^k + (1 - r)^k)`. A `k` above 1.0 sharpens the value
/// toward the near extreme. A `k` under 1.0 softens the value toward
/// 0.5. Because the transform keeps the order and the side of 0.5, the
/// accuracy of the miner does not change. Only the calibration of its
/// reported confidence changes.
///
/// The function returns `r` unchanged at the two ends, 0.0 and 1.0, to
/// side-step a `0.0 / 0.0` or a negative-base power at the edge.
fn sharpen_toward_extreme(r: f64, k: f64) -> f64 {
    if r <= 0.0 {
        0.0
    } else if r >= 1.0 {
        1.0
    } else {
        let hi = r.powf(k);
        let lo = (1.0 - r).powf(k);
        hi / (hi + lo)
    }
}

/// This function builds the response text and the response value for
/// one well formed answer.
///
/// The function clamps `confidence` into the range 0.0 up to 1.0
/// first, so the JSON text and the recorded confidence always agree.
/// The function uses `serde_json` to build the text, so the float
/// round-trips to the exact same bits when a reader parses it back.
fn make_answer_response(confidence: f64, correct: bool) -> Response {
    let clamped = confidence.clamp(0.0, 1.0);
    let body = serde_json::json!({ "confidence": clamped });
    let json = body.to_string();
    Response {
        json,
        kind: ResponseKind::Answer {
            confidence: clamped,
            correct,
        },
    }
}

/// This function makes the `Oracle` response for one item.
///
/// `Oracle` is always correct. It reports 0.99 when the label is 1,
/// and 0.01 when the label is 0. The function draws no random value,
/// because `Oracle` needs no randomness to be always right.
fn oracle_response(item: &Item) -> Response {
    let confidence = if item.label == 1 { 0.99 } else { 0.01 };
    make_answer_response(confidence, true)
}

/// This function makes the `ConstantMajority` response for one item.
///
/// `ConstantMajority` always predicts the majority label of the whole
/// data set. It ignores the item completely, other than to check if
/// the prediction agrees with this item's true label. It reports 0.99
/// when the majority label is 1, and 0.01 when the majority label is
/// 0.
fn constant_majority_response(item: &Item, dataset: &Dataset) -> Response {
    let confidence = if dataset.majority_label == 1 {
        0.99
    } else {
        0.01
    };
    let correct = dataset.majority_label == item.label;
    make_answer_response(confidence, correct)
}

/// This function makes the `Random` response for one item.
///
/// `Random` reports a uniform random confidence that carries no
/// information about the item. The answer counts as correct when the
/// reported value falls on the correct side of 0.5.
fn random_response(item: &Item, rng: &mut Rng) -> Response {
    let reported = rng.next_f64();
    let correct = (reported > 0.5) == (item.label == 1);
    make_answer_response(reported, correct)
}

/// This function makes the `Abstainer` response for one item.
///
/// `Abstainer` does not answer the hardest 30 percent of the data set:
/// every item with a signal at or under the data set's
/// `hard_signal_threshold`. On those items the function returns the
/// `Abstain` response, with body `"{}"`. On every other item
/// `Abstainer` behaves like `Oracle`: always correct, certainty 0.99.
fn abstainer_response(item: &Item, dataset: &Dataset, _rng: &mut Rng) -> Response {
    if item.signal <= dataset.hard_signal_threshold {
        Response {
            json: "{}".to_string(),
            kind: ResponseKind::Abstain,
        }
    } else {
        oracle_response(item)
    }
}

/// This function makes the `Malformer` response for one item.
///
/// `Malformer` behaves like `NoisyGood` on most items. On every item
/// where `item.index % 10 == 0`, the function sends a malformed body
/// instead. The five malformed bodies rotate by
/// `(item.index / 10) % 5`, so the same index always gives the same
/// bad text.
///
/// A malformed item draws no random value from `rng`. The bad text
/// comes only from the item index, so it needs no randomness. This
/// choice is on purpose and stays the same every time: `rng` only
/// advances on the well formed items, never on the malformed ones.
fn malformer_response(item: &Item, rng: &mut Rng) -> Response {
    if item.index.is_multiple_of(10) {
        let form_index = (item.index / 10) % MALFORMED_FORMS.len();
        Response {
            json: MALFORMED_FORMS[form_index].to_string(),
            kind: ResponseKind::Malformed,
        }
    } else {
        let (reported, correct) = calibrated_answer(0.85, item, rng);
        make_answer_response(reported, correct)
    }
}

/// This function makes the `Contrarian` response for one item.
///
/// `Contrarian` starts from the same calibrated core as `NoisyGood`,
/// at target accuracy 0.85, then inverts the reported probability:
/// `reported = 1.0 - r`. The inversion flips the predicted label every
/// time, so `Contrarian`'s accuracy is `1.0` minus the mean accuracy of
/// the plain calibrated core on this data set.
///
/// That mean accuracy is under 0.85 in general, because
/// `calibrated_answer` damps the target accuracy by the item signal.
/// On a data set with a uniform signal, such as `Balanced`, the mean
/// signal is 0.5, so the mean accuracy of the calibrated core is near
/// `0.5 + (0.85 - 0.5) * 0.5 = 0.675`, and `Contrarian`'s accuracy is
/// near `1.0 - 0.675 = 0.325`, not near 0.15.
fn contrarian_response(item: &Item, rng: &mut Rng) -> Response {
    let (r, _correct) = calibrated_answer(0.85, item, rng);
    let reported = 1.0 - r;
    let correct = (reported > 0.5) == (item.label == 1);
    make_answer_response(reported, correct)
}

/// This function applies the class base rate to a signal-conditional
/// confidence.
///
/// This function models a miner that has calibrated against the
/// historical class balance of the intent. A real miner can read this
/// balance from past labelled data, so this is a realistic thing for a
/// real miner to do.
///
/// The function turns a confidence `c` that carries only the item
/// signal into the true posterior probability of label 1, given a class
/// base rate `base_rate` of `B`:
///
/// ```text
/// posterior = (c * B) / (c * B + (1 - c) * (1 - B))
/// ```
///
/// This is Bayes' rule. `c` stands for `P(reported side | label = 1)`
/// scaled against a base rate of 0.5, and the formula moves that value
/// to the true posterior at the real base rate `B`.
///
/// # Degenerate cases
///
/// - `base_rate` at or under 0.0 gives 0.0. `base_rate` at or over 1.0
///   gives 1.0. Every item shares one label at these two base rates, so
///   the posterior does not depend on `confidence`.
/// - A denominator at or near 0.0 gives 0.5. This function never
///   returns `NaN`.
///
/// The function clamps `confidence` and the result into the range 0.0
/// up to 1.0.
///
/// # The base rate 0.5 identity
///
/// At `base_rate == 0.5`, `posterior == confidence`. The proof: the
/// formula becomes `(c * 0.5) / (c * 0.5 + (1 - c) * 0.5)`, and the
/// factor of 0.5 cancels top and bottom, leaving `c`. This is why
/// [`Archetype::BayesCalibratedGood`] gives the same answers as
/// `NoisyGood` on a data set with a base rate of 0.5.
#[must_use]
pub fn bayes_posterior(confidence: f64, base_rate: f64) -> f64 {
    if base_rate <= 0.0 {
        return 0.0;
    }
    if base_rate >= 1.0 {
        return 1.0;
    }
    let c = confidence.clamp(0.0, 1.0);
    let b = base_rate;
    let numerator = c * b;
    let denominator = numerator + (1.0 - c) * (1.0 - b);
    if denominator <= 0.0 {
        return 0.5;
    }
    (numerator / denominator).clamp(0.0, 1.0)
}

/// This function makes the `BayesCalibratedGood` response for one item.
///
/// The function draws the same signal-conditional core as `NoisyGood`,
/// at target accuracy 0.85, then applies [`bayes_posterior`] with the
/// data set's [`Dataset::realised_base_rate`] as the prior. The
/// posterior can fall on the other side of 0.5 from the plain core's
/// reported value, on an item near the decision boundary. That flip is
/// correct Bayesian behaviour, not a bug: a miner that knows the class
/// is skewed toward label 1 should lean toward 1 on a borderline item.
/// So this function reads correctness from the final posterior, the
/// same way `random_response` and `contrarian_response` do, not from
/// the plain core's own correctness flag.
fn bayes_calibrated_good_response(item: &Item, dataset: &Dataset, rng: &mut Rng) -> Response {
    let (reported, _core_correct) = calibrated_answer(0.85, item, rng);
    let posterior = bayes_posterior(reported, dataset.realised_base_rate);
    let correct = (posterior > 0.5) == (item.label == 1);
    make_answer_response(posterior, correct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset;
    use crate::types::DatasetShape;

    fn one_item(index: usize, label: u8, signal: f64) -> Item {
        Item {
            index,
            label,
            signal,
        }
    }

    #[test]
    fn oracle_is_always_correct() {
        let mut rng = Rng::new(1);
        for label in [0u8, 1u8] {
            let item = one_item(0, label, 0.5);
            let response = oracle_response(&item);
            match response.kind {
                ResponseKind::Answer { correct, .. } => assert!(correct),
                _ => panic!("oracle must give an answer"),
            }
            let _ = &mut rng;
        }
    }

    #[test]
    fn constant_majority_ignores_the_item_signal() {
        let dataset = dataset::generate(DatasetShape::Balanced, 200, 9);
        let item_a = one_item(0, dataset.majority_label, 0.0);
        let item_b = one_item(1, dataset.majority_label, 1.0);
        let response_a = constant_majority_response(&item_a, &dataset);
        let response_b = constant_majority_response(&item_b, &dataset);
        assert_eq!(response_a.json, response_b.json);
    }

    #[test]
    fn calibrated_answer_guard_does_not_panic_near_zero_signal() {
        let mut rng = Rng::new(5);
        let item = one_item(0, 1, 0.0);
        let (reported, _correct) = calibrated_answer(0.85, &item, &mut rng);
        assert!((0.0..=1.0).contains(&reported));
    }

    #[test]
    fn sharpen_keeps_the_fixed_point_at_one_half() {
        assert!((sharpen_toward_extreme(0.5, 2.5) - 0.5).abs() < 1e-9);
        assert!((sharpen_toward_extreme(0.5, 0.4) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn sharpen_pushes_high_values_higher() {
        let sharpened = sharpen_toward_extreme(0.7, 2.5);
        assert!(sharpened > 0.7);
    }

    #[test]
    fn sharpen_pulls_high_values_toward_half() {
        let softened = sharpen_toward_extreme(0.7, 0.4);
        assert!(softened < 0.7 && softened > 0.5);
    }

    #[test]
    fn sharpen_handles_the_edges() {
        assert_eq!(sharpen_toward_extreme(0.0, 2.5), 0.0);
        assert_eq!(sharpen_toward_extreme(1.0, 2.5), 1.0);
    }

    #[test]
    fn abstainer_abstains_on_the_hardest_items_only() {
        let dataset = dataset::generate(DatasetShape::Balanced, 2_000, 11);
        let mut rng = Rng::new(1);
        let mut abstains = 0usize;
        for item in &dataset.items {
            let response = respond(Archetype::Abstainer, item, &dataset, &mut rng);
            if matches!(response.kind, ResponseKind::Abstain) {
                abstains += 1;
                assert!(item.signal <= dataset.hard_signal_threshold);
            }
        }
        let rate = abstains as f64 / dataset.items.len() as f64;
        assert!((rate - 0.30).abs() < 0.05, "abstain rate was {rate}");
    }

    #[test]
    fn malformer_sends_bad_text_on_every_tenth_item() {
        // The response seed (1012) differs from the data set seed
        // (12) on purpose. See the trap note on `responses_for`.
        let dataset = dataset::generate(DatasetShape::Balanced, 100, 12);
        let responses = responses_for(
            Archetype::Malformer,
            &dataset,
            ResponseSeed::new_unchecked(1012),
        );
        for (i, response) in responses.iter().enumerate() {
            if i.is_multiple_of(10) {
                assert!(matches!(response.kind, ResponseKind::Malformed));
                let expected = MALFORMED_FORMS[(i / 10) % MALFORMED_FORMS.len()];
                assert_eq!(response.json, expected);
            } else {
                assert!(matches!(response.kind, ResponseKind::Answer { .. }));
            }
        }
    }

    #[test]
    fn contrarian_accuracy_matches_one_minus_noisy_good_accuracy() {
        // The response seed (1013) must differ from the data set seed
        // (13). See the trap note on `responses_for`: a matching seed
        // lines up the miner draws with the item's own signal draw and
        // gives a biased accuracy number.
        //
        // The accuracy here is near 0.325, not near 0.15. Here is why.
        // `calibrated_answer` damps the target accuracy `a` by the item
        // signal: `a_eff = 0.5 + (a - 0.5) * signal`. On the balanced
        // data set the signal is uniform on 0.0 up to 1.0, so the mean
        // signal is 0.5, and the mean accuracy of a plain calibrated
        // miner at `a = 0.85` is near `0.5 + 0.35 * 0.5 = 0.675`, not
        // 0.85. `Contrarian` inverts the reported value, which flips
        // the predicted label every time (the reported value never
        // lands on exactly 0.5), so `Contrarian`'s accuracy is near
        // `1.0 - 0.675 = 0.325`. This number, not 0.15, is the correct
        // target for a miner built by inverting this exact core.
        let dataset = dataset::generate(DatasetShape::Balanced, 20_000, 13);
        let responses = responses_for(
            Archetype::Contrarian,
            &dataset,
            ResponseSeed::new_unchecked(1013),
        );
        let correct = responses
            .iter()
            .filter(|r| matches!(r.kind, ResponseKind::Answer { correct: true, .. }))
            .count();
        let rate = correct as f64 / responses.len() as f64;
        let expected = 1.0 - (0.5 + 0.35 * 0.5);
        assert!(
            (rate - expected).abs() < 0.03,
            "contrarian accuracy was {rate}, expected near {expected}"
        );
    }

    #[test]
    fn every_confidence_stays_in_range() {
        let dataset = dataset::generate(DatasetShape::HardTail, 500, 14);
        for archetype in Archetype::ALL {
            let responses = responses_for(archetype, &dataset, ResponseSeed::new_unchecked(1014));
            for response in &responses {
                if let ResponseKind::Answer { confidence, .. } = response.kind {
                    assert!(
                        (0.0..=1.0).contains(&confidence),
                        "{archetype:?} gave {confidence}"
                    );
                }
            }
        }
    }

    #[test]
    fn bayes_posterior_identity_at_base_rate_one_half() {
        for c in [0.0, 0.1, 0.3, 0.5, 0.7, 0.85, 0.99, 1.0] {
            let posterior = bayes_posterior(c, 0.5);
            assert!(
                (posterior - c).abs() < 1e-9,
                "bayes_posterior({c}, 0.5) was {posterior}, expected {c}"
            );
        }
    }

    #[test]
    fn bayes_posterior_handles_the_degenerate_base_rates() {
        for c in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(bayes_posterior(c, 0.0), 0.0, "base_rate 0.0, c {c}");
            assert_eq!(bayes_posterior(c, 1.0), 1.0, "base_rate 1.0, c {c}");
        }
        // c = 0.0 and base_rate = 1.0 makes both terms of the
        // denominator zero. The function must not return NaN.
        let edge = bayes_posterior(0.0, 1.0);
        assert!(!edge.is_nan());
        assert_eq!(edge, 1.0);
    }

    #[test]
    fn bayes_posterior_never_returns_nan_and_stays_in_range() {
        let confidences = [0.0, 0.01, 0.3, 0.5, 0.7, 0.99, 1.0];
        let base_rates = [0.0, 0.1, 0.5, 0.9, 1.0];
        for &c in &confidences {
            for &b in &base_rates {
                let posterior = bayes_posterior(c, b);
                assert!(!posterior.is_nan(), "bayes_posterior({c}, {b}) was NaN");
                assert!(
                    (0.0..=1.0).contains(&posterior),
                    "bayes_posterior({c}, {b}) was {posterior}, out of range"
                );
            }
        }
    }

    #[test]
    fn bayes_posterior_moves_toward_the_base_rate_prior() {
        // A base rate above 0.5 must pull the posterior above the raw
        // confidence, and a base rate below 0.5 must pull it below.
        let c = 0.6;
        assert!(bayes_posterior(c, 0.9) > c);
        assert!(bayes_posterior(c, 0.1) < c);
    }

    #[test]
    fn bayes_calibrated_good_equals_noisy_good_at_base_rate_one_half() {
        // The identity `B = 0.5` gives `posterior == c`, so on a data
        // set with an exact base rate of 0.5 this archetype must give
        // the same reported confidence as `NoisyGood`, item for item.
        // `generate` gives a realised base rate close to, but not
        // exactly, 0.5, so this test overrides that one field to force
        // the exact identity case. Every other field of the data set
        // stays as generated.
        //
        // This test checks the reported confidence only, not the
        // `correct` flag. A few items with a signal near 0.0 make
        // `gamma` in `calibrated_answer` so large that `u.powf(gamma)`
        // underflows to 0.0, so `c` lands on exactly 0.5. At that exact
        // tie, `reported` is 0.5 no matter which side was predicted, so
        // it carries no information about which side the hidden
        // bernoulli draw picked. `NoisyGood`'s `correct` flag comes
        // straight from that hidden draw. `BayesCalibratedGood`'s
        // `correct` flag comes from the sign of the posterior, which is
        // ambiguous at this one exact float value. The two are allowed
        // to differ only there. Confidence is the value a validator
        // scores, so confidence is the value this test must match.
        let mut dataset = dataset::generate(DatasetShape::Balanced, 5_000, 4_242);
        dataset.realised_base_rate = 0.5;

        let seed = ResponseSeed::derive(dataset.seed);
        let noisy_good = responses_for(Archetype::NoisyGood, &dataset, seed);
        let bayes_good = responses_for(Archetype::BayesCalibratedGood, &dataset, seed);

        assert_eq!(noisy_good.len(), bayes_good.len());
        for (a, b) in noisy_good.iter().zip(bayes_good.iter()) {
            match (&a.kind, &b.kind) {
                (
                    ResponseKind::Answer { confidence: ca, .. },
                    ResponseKind::Answer { confidence: cb, .. },
                ) => {
                    assert!((ca - cb).abs() < 1e-9, "confidence differed: {ca} vs {cb}");
                }
                _ => panic!("both archetypes must give a well formed answer here"),
            }
        }
    }

    #[test]
    fn bayes_calibrated_good_leans_on_the_skewed_prior() {
        // On a skewed data set, an item with a weak, near-coin-flip
        // signal-conditional confidence must still lean toward the
        // majority label once the prior is applied.
        let dataset = dataset::generate(DatasetShape::Skewed, 5_000, 4_243);
        let seed = ResponseSeed::derive(dataset.seed);
        let responses = responses_for(Archetype::BayesCalibratedGood, &dataset, seed);
        let mean_confidence: f64 = responses
            .iter()
            .filter_map(|r| match r.kind {
                ResponseKind::Answer { confidence, .. } => Some(confidence),
                _ => None,
            })
            .sum::<f64>()
            / responses.len() as f64;
        // The skewed data set has a base rate near 0.90, so the mean
        // reported confidence must sit well above 0.5.
        assert!(
            mean_confidence > 0.6,
            "mean confidence was {mean_confidence}, expected it to lean toward the base rate"
        );
    }
}
