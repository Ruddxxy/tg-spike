//! This test file checks that the simulator gives the exact same
//! output for the exact same input, every time, and that a change of
//! seed gives different output for a miner that draws random values.

use miner_sim::archetype::responses_for;
use miner_sim::dataset;
use miner_sim::types::{Archetype, DatasetShape, ResponseKind, ResponseSeed};

/// The data set seed for the byte-identical checks.
const DATASET_SEED: u64 = 7_001;
/// The response seed for the byte-identical checks.
///
/// This value must differ from `DATASET_SEED`. See the trap note on
/// `miner_sim::archetype::responses_for`.
const RESPONSE_SEED: u64 = 909_007_001;
/// A second response seed, used to check that a seed change gives a
/// different output. This value must also differ from `DATASET_SEED`.
const OTHER_RESPONSE_SEED: u64 = 909_007_002;

/// This function joins the `json` field of every response into one
/// string, with a separator that cannot appear inside a JSON body or a
/// malformed body. The joined string is a compact stand-in for
/// "the whole output, byte for byte".
fn join_json(responses: &[miner_sim::types::Response]) -> String {
    responses
        .iter()
        .map(|r| r.json.as_str())
        .collect::<Vec<_>>()
        .join("\u{1}")
}

#[test]
fn same_seed_gives_byte_identical_responses_for_every_archetype_and_shape() {
    for shape in DatasetShape::ALL {
        let dataset = dataset::generate(shape, 3_000, DATASET_SEED);
        for archetype in Archetype::ALL {
            // Run the check twice in this same process. A generator
            // with hidden shared state, such as a static counter,
            // would fail this check even though a fresh process would
            // pass it by luck.
            let seed = ResponseSeed::new_unchecked(RESPONSE_SEED);
            let first = responses_for(archetype, &dataset, seed);
            let second = responses_for(archetype, &dataset, seed);
            assert_eq!(
                join_json(&first),
                join_json(&second),
                "{archetype:?} on {shape:?} gave different output on a repeat run with the same seed"
            );
        }
    }
}

#[test]
fn different_seed_gives_different_output_for_stochastic_archetypes() {
    // These archetypes draw at least one random value per item, on at
    // least some items, so a seed change must change their output.
    // `Oracle`, `ConstantMajority`, and `Abstainer` draw no random
    // value at all, so a seed change would not move them; this test
    // does not check those three archetypes for this reason.
    let stochastic = [
        Archetype::NoisyGood,
        Archetype::NoisyMediocre,
        Archetype::Random,
        Archetype::OverconfidentGood,
        Archetype::UnderconfidentGood,
        Archetype::Malformer,
        Archetype::Contrarian,
        Archetype::BayesCalibratedGood,
    ];

    let dataset = dataset::generate(DatasetShape::Balanced, 3_000, DATASET_SEED);
    for archetype in stochastic {
        let first = responses_for(
            archetype,
            &dataset,
            ResponseSeed::new_unchecked(RESPONSE_SEED),
        );
        let second = responses_for(
            archetype,
            &dataset,
            ResponseSeed::new_unchecked(OTHER_RESPONSE_SEED),
        );
        assert_ne!(
            join_json(&first),
            join_json(&second),
            "{archetype:?} gave the same output for two different response seeds"
        );
    }
}

#[test]
fn generate_gives_identical_dataset_for_the_same_seed() {
    for shape in DatasetShape::ALL {
        // Run the check twice in this same process, for the same
        // reason as the response check above.
        let first = dataset::generate(shape, 4_000, DATASET_SEED);
        let second = dataset::generate(shape, 4_000, DATASET_SEED);
        assert_eq!(first.majority_label, second.majority_label);
        assert_eq!(first.hard_signal_threshold, second.hard_signal_threshold);
        for (a, b) in first.items.iter().zip(second.items.iter()) {
            assert_eq!(
                a.label, b.label,
                "{shape:?} gave a different label on a repeat run"
            );
            assert_eq!(
                a.signal, b.signal,
                "{shape:?} gave a different signal on a repeat run"
            );
        }
    }
}

#[test]
fn a_change_of_prng_output_would_be_caught_by_a_hardcoded_prefix() {
    // This is a known-answer test, not a statistical one. The values
    // below came from one real run of this exact code. A change to
    // the PRNG, to `calibrated_answer`, or to the JSON format would
    // move at least one of these five values, and this test would
    // fail. That failure is the point: the other tests in this file
    // check consistency between two runs, but this test checks the
    // exact numbers do not drift.
    let dataset = dataset::generate(DatasetShape::Balanced, 5, 1);
    let responses = responses_for(
        Archetype::NoisyGood,
        &dataset,
        ResponseSeed::new_unchecked(2),
    );

    let jsons: Vec<&str> = responses.iter().map(|r| r.json.as_str()).collect();
    assert_eq!(
        jsons,
        vec![
            "{\"confidence\":0.7607587160994493}",
            "{\"confidence\":0.5285853711813648}",
            "{\"confidence\":0.4535408555458772}",
            "{\"confidence\":0.2762174992806201}",
            "{\"confidence\":0.8783997048787439}",
        ]
    );

    for response in &responses {
        assert!(matches!(response.kind, ResponseKind::Answer { .. }));
    }
}
