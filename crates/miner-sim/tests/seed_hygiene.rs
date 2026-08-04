//! This test file checks that the response seed does not collide with
//! the data set seed.
//!
//! A past defect used the same seed and the same PRNG for the data set
//! and for the miner responses. Each item draw of the data set drew 2
//! values (label, signal). Each item draw of `calibrated_answer` also
//! drew up to 2 values (the certainty draw `u`, then the correctness
//! draw). A matching seed lined up a miner's correctness draw with the
//! exact random value that made that same item's signal. Correctness
//! became an ANTI-correlated function of item difficulty, backwards
//! from the true relationship. Every other test still passed.
//!
//! This file locks in two facts:
//!
//! 1. [`ResponseSeed::derive`] gives a POSITIVE correlation between item
//!    signal and miner correctness, close to the value that the design
//!    of `calibrated_answer` predicts. A calibrated miner IS more often
//!    correct on an easy (high signal) item, by design. See the doc
//!    comment on `calibrated_answer`: `a_eff = 0.5 + (a - 0.5) *
//!    item.signal`. So the honest correlation is positive, not zero.
//! 2. A deliberately colliding seed, built with
//!    [`ResponseSeed::new_unchecked`] from the data set's own seed,
//!    gives a MATERIALLY DIFFERENT correlation: a negative one. This is
//!    the real regression test. It proves the collision is detectable,
//!    and that `derive` avoids it.

use miner_sim::archetype::responses_for;
use miner_sim::dataset;
use miner_sim::types::{Archetype, Dataset, DatasetShape, ResponseKind, ResponseSeed};

/// The item count for the correlation check. A large count keeps the
/// sampling noise in the correlation estimate small.
const ITEM_COUNT: usize = 100_000;

/// The seed of the balanced data set.
const DATASET_SEED: u64 = 5_301;

/// This function gives the Pearson correlation of two equal length
/// slices.
///
/// The function returns 0.0 when either slice has zero variance. A
/// zero-variance slice would otherwise divide by zero and give `NaN`.
fn pearson_correlation(xs: &[f64], ys: &[f64]) -> f64 {
    assert_eq!(xs.len(), ys.len(), "the two slices must have equal length");
    let n = xs.len() as f64;
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = ys.iter().sum::<f64>() / n;

    let mut cov = 0.0;
    let mut var_x = 0.0;
    let mut var_y = 0.0;
    for (x, y) in xs.iter().zip(ys.iter()) {
        let dx = x - mean_x;
        let dy = y - mean_y;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }
    if var_x <= 0.0 || var_y <= 0.0 {
        return 0.0;
    }
    cov / (var_x.sqrt() * var_y.sqrt())
}

/// This function gives the item signal and the miner correctness of
/// every well formed `NoisyGood` answer on `dataset`, using
/// `response_seed`.
///
/// The two returned vectors line up index for index: `signals[i]` and
/// `corrects[i]` describe the same response.
fn signal_and_correctness(dataset: &Dataset, response_seed: ResponseSeed) -> (Vec<f64>, Vec<f64>) {
    let responses = responses_for(Archetype::NoisyGood, dataset, response_seed);
    let mut signals = Vec::with_capacity(dataset.items.len());
    let mut corrects = Vec::with_capacity(dataset.items.len());
    for (item, response) in dataset.items.iter().zip(responses.iter()) {
        if let ResponseKind::Answer { correct, .. } = response.kind {
            signals.push(item.signal);
            corrects.push(if correct { 1.0 } else { 0.0 });
        }
    }
    (signals, corrects)
}

#[test]
fn derived_seed_gives_positive_correlation_and_a_colliding_seed_does_not() {
    let dataset = dataset::generate(DatasetShape::Balanced, ITEM_COUNT, DATASET_SEED);

    let derived_seed = ResponseSeed::derive(DATASET_SEED);
    let (derived_signals, derived_corrects) = signal_and_correctness(&dataset, derived_seed);
    let derived_correlation = pearson_correlation(&derived_signals, &derived_corrects);

    // This seed matches the data set seed on purpose. It rebuilds the
    // exact collision that the old defect had. `new_unchecked` states
    // that risk out loud, at the call site, which is the whole point of
    // the newtype.
    let colliding_seed = ResponseSeed::new_unchecked(dataset.seed);
    let (colliding_signals, colliding_corrects) = signal_and_correctness(&dataset, colliding_seed);
    let colliding_correlation = pearson_correlation(&colliding_signals, &colliding_corrects);

    println!("derived seed correlation:   {derived_correlation:.4}");
    println!("colliding seed correlation: {colliding_correlation:.4}");

    // The design of `calibrated_answer` gives an exact linear relation
    // between item signal and the CHANCE of correctness:
    // `E[correct | signal] = 0.5 + (a - 0.5) * signal`, with `a = 0.85`
    // for `NoisyGood`. With `signal` uniform on 0.0 up to 1.0, this
    // works out to an expected Pearson correlation near 0.216. The
    // bound below gives wide margin around that value: at 100,000
    // items the sampling noise in the correlation estimate is far
    // smaller than this margin.
    assert!(
        derived_correlation > 0.15,
        "derived seed correlation was {derived_correlation:.4}, expected a clear positive value near 0.22"
    );
    assert!(
        derived_correlation < 0.30,
        "derived seed correlation was {derived_correlation:.4}, higher than the expected range"
    );

    // The old defect made correctness an ANTI-correlated function of
    // signal: a high signal item drew its own signal value as the
    // miner's correctness draw, so a high signal (a value close to
    // 1.0) made the correctness bernoulli draw LESS likely to land
    // under the certainty threshold. The colliding correlation must be
    // negative.
    assert!(
        colliding_correlation < 0.0,
        "colliding seed correlation was {colliding_correlation:.4}, expected a negative value"
    );

    // This is the real regression test. A future change that quietly
    // drops `ResponseSeed` in favour of a raw `u64`, and passes a
    // matching seed by accident, must show up here as a correlation
    // that is no longer close to the derived one.
    let diff = (derived_correlation - colliding_correlation).abs();
    assert!(
        diff > 0.10,
        "derived correlation {derived_correlation:.4} and colliding correlation \
         {colliding_correlation:.4} were too close; the seed collision must be detectable"
    );
}
