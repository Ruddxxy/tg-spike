//! Integration tests for the miner simulator.
//!
//! Each test builds the balanced data set, scores every archetype
//! through the compiled WASM module, and checks a property of the
//! whole pipeline. Every test skips with a printed message, and does
//! not fail, when the WASM file is absent. This lets the crate build
//! and test before `eval-script` is built for `wasm32-unknown-unknown`.
//!
//! # Score direction
//!
//! A HIGH score is good. Read the note on score direction in
//! `types.rs`.
//!
//! # The invariants under ejection
//!
//! Invariants 6 and 7 (abstainer, malformer) do not test a rank number
//! under the `Eject` aggregation model. They test the AGGREGATION
//! layer: does the protocol remove the failing miner from the pool, at
//! the first item that failed. `verdict::check_all_eject` checks that
//! the archetype has no rank and that the recorded ejection reason is
//! right. This file goes one step further and checks that the recorded
//! `first_failure.index` really is the index of the FIRST abstained or
//! malformed response, by re-scanning the raw response list. `Standings`
//! does not carry the raw response list, so this check can only run
//! here, where the response list is still in scope.

use host_runner::instance::ScriptInstance;
use miner_sim::types::{
    AggregationModel, Archetype, DatasetShape, EjectionReason, Metric, ResponseKind, ResponseSeed,
};
use miner_sim::{archetype, bootstrap, dataset, leaderboard, scoring, verdict};

const ITEMS: usize = 500;
const SEED: u64 = 0xC0FFEE;

/// This function loads the WASM module, or returns `None` with a
/// printed skip message when the file is absent or fails to load.
fn load_instance_or_skip(test_name: &str) -> Option<ScriptInstance> {
    let wasm_path = scoring::resolve_wasm_path();
    if !wasm_path.exists() {
        println!("skip {test_name}: no wasm file at {}", wasm_path.display());
        return None;
    }
    match ScriptInstance::load(&wasm_path) {
        Ok(instance) => Some(instance),
        Err(e) => {
            println!("skip {test_name}: load failed: {e}");
            None
        }
    }
}

/// This function scores every archetype on the balanced data set, with
/// one metric, and returns the leaderboard rows and the raw miner
/// results.
fn score_balanced(
    instance: &mut ScriptInstance,
    metric: Metric,
) -> (
    Vec<miner_sim::types::LeaderboardRow>,
    Vec<miner_sim::types::MinerResult>,
) {
    let ds = dataset::generate(DatasetShape::Balanced, ITEMS, SEED);
    let response_seed = ResponseSeed::derive(ds.seed);
    let mut results = Vec::with_capacity(Archetype::ALL.len());
    for arch in Archetype::ALL {
        let responses = archetype::responses_for(arch, &ds, response_seed);
        let result = scoring::score_miner(instance, &ds, arch, &responses, metric)
            .expect("scoring must not fail in this test");
        results.push(result);
    }
    let rows = leaderboard::build(&results);
    (rows, results)
}

#[test]
fn seven_invariants_hold_on_balanced_brier_score_and_keep() {
    let Some(mut instance) =
        load_instance_or_skip("seven_invariants_hold_on_balanced_brier_score_and_keep")
    else {
        return;
    };

    let (rows, _results) = score_balanced(&mut instance, Metric::Brier);
    let lines = verdict::check_all(&rows);

    for line in &lines {
        assert!(
            line.passed,
            "invariant {} failed: {} | {}",
            line.number, line.statement, line.detail
        );
    }
    assert!(verdict::all_passed(&lines));
}

/// This test checks the same seven invariants under the `Eject`
/// aggregation model, which is the real protocol rule. Invariants 6 and
/// 7 are restated here: they check that the protocol ejected the
/// abstainer and the malformer, not that either one holds a low rank.
#[test]
fn seven_invariants_hold_on_balanced_brier_eject() {
    let Some(mut instance) = load_instance_or_skip("seven_invariants_hold_on_balanced_brier_eject")
    else {
        return;
    };

    let (_rows, results) = score_balanced(&mut instance, Metric::Brier);
    let standings = leaderboard::build_standings(&results, AggregationModel::Eject);
    let lines = verdict::check_all_eject(&standings);

    for line in &lines {
        assert!(
            line.passed,
            "invariant {} failed: {} | {}",
            line.number, line.statement, line.detail
        );
    }
    assert!(verdict::all_passed(&lines));
}

/// This test checks that the recorded `first_failure.index` of the
/// abstainer really is the index of its FIRST abstained response, and
/// not merely some abstained response.
///
/// This test re-derives the raw response list, the same way
/// `score_balanced` does, and scans it by hand. `Standings` does not
/// carry the raw response list, so `verdict::check_all_eject` cannot
/// run this check on its own.
#[test]
fn abstainer_first_failure_index_is_the_first_abstention() {
    let Some(mut instance) =
        load_instance_or_skip("abstainer_first_failure_index_is_the_first_abstention")
    else {
        return;
    };

    let ds = dataset::generate(DatasetShape::Balanced, ITEMS, SEED);
    let response_seed = ResponseSeed::derive(ds.seed);
    let responses = archetype::responses_for(Archetype::Abstainer, &ds, response_seed);

    let true_first_index = responses
        .iter()
        .zip(ds.items.iter())
        .find(|(response, _)| matches!(response.kind, ResponseKind::Abstain))
        .map(|(_, item)| item.index);

    let result = scoring::score_miner(
        &mut instance,
        &ds,
        Archetype::Abstainer,
        &responses,
        Metric::Brier,
    )
    .expect("scoring must not fail in this test");

    match (result.first_failure, true_first_index) {
        (Some(failure), Some(expected_index)) => {
            assert_eq!(failure.index, expected_index);
            assert_eq!(failure.reason, EjectionReason::NoResponse);
        }
        (None, None) => {
            // The abstainer never abstained on this data set. This can
            // happen only if no item fell at or under the hard signal
            // threshold, which the data set generator does not allow
            // for a data set this size. Fail loudly instead of passing
            // by accident.
            panic!("abstainer did not abstain on this data set, and no failure was expected either; the test fixture assumption broke");
        }
        (recorded, expected) => panic!(
            "first_failure mismatch: recorded {recorded:?}, expected an abstention at {expected:?}"
        ),
    }
}

/// This test checks that the recorded `first_failure.index` of the
/// malformer really is the index of its FIRST malformed response.
#[test]
fn malformer_first_failure_index_is_the_first_malformed_response() {
    let Some(mut instance) =
        load_instance_or_skip("malformer_first_failure_index_is_the_first_malformed_response")
    else {
        return;
    };

    let ds = dataset::generate(DatasetShape::Balanced, ITEMS, SEED);
    let response_seed = ResponseSeed::derive(ds.seed);
    let responses = archetype::responses_for(Archetype::Malformer, &ds, response_seed);

    let true_first_index = responses
        .iter()
        .zip(ds.items.iter())
        .find(|(response, _)| matches!(response.kind, ResponseKind::Malformed))
        .map(|(_, item)| item.index);

    let result = scoring::score_miner(
        &mut instance,
        &ds,
        Archetype::Malformer,
        &responses,
        Metric::Brier,
    )
    .expect("scoring must not fail in this test");

    match (result.first_failure, true_first_index) {
        (Some(failure), Some(expected_index)) => {
            assert_eq!(failure.index, expected_index);
            assert_eq!(failure.reason, EjectionReason::MalformedResponse);
        }
        (None, None) => {
            panic!("malformer sent no malformed response on this data set; the test fixture assumption broke");
        }
        (recorded, expected) => panic!(
            "first_failure mismatch: recorded {recorded:?}, expected a malformed response at {expected:?}"
        ),
    }
}

/// This test checks that the `Eject` model removes the abstainer and
/// the malformer from the ranked list, and lists them in `ejected` with
/// the right reason. This is the aggregation-layer half of the old
/// invariants 6 and 7.
#[test]
fn eject_model_removes_abstainer_and_malformer_from_the_ranked_pool() {
    let Some(mut instance) =
        load_instance_or_skip("eject_model_removes_abstainer_and_malformer_from_the_ranked_pool")
    else {
        return;
    };

    let (_rows, results) = score_balanced(&mut instance, Metric::Brier);
    let standings = leaderboard::build_standings(&results, AggregationModel::Eject);

    assert!(
        !standings
            .ranked
            .iter()
            .any(|row| row.archetype == Archetype::Abstainer),
        "the abstainer must not hold a rank under the eject model"
    );
    assert!(
        !standings
            .ranked
            .iter()
            .any(|row| row.archetype == Archetype::Malformer),
        "the malformer must not hold a rank under the eject model"
    );

    let abstainer_ejection = standings
        .ejected
        .iter()
        .find(|row| row.archetype == Archetype::Abstainer)
        .expect("abstainer must be in the ejected list");
    assert_eq!(abstainer_ejection.reason, EjectionReason::NoResponse);

    let malformer_ejection = standings
        .ejected
        .iter()
        .find(|row| row.archetype == Archetype::Malformer)
        .expect("malformer must be in the ejected list");
    assert_eq!(malformer_ejection.reason, EjectionReason::MalformedResponse);
}

#[test]
fn full_report_is_byte_identical_for_the_same_seed() {
    let Some(mut instance_a) =
        load_instance_or_skip("full_report_is_byte_identical_for_the_same_seed")
    else {
        return;
    };
    let Some(mut instance_b) = load_instance_or_skip("(second instance)") else {
        return;
    };

    let (rows_a, results_a) = score_balanced(&mut instance_a, Metric::Brier);
    let (rows_b, results_b) = score_balanced(&mut instance_b, Metric::Brier);

    let text_a = leaderboard::render(&rows_a, "BRIER LEADERBOARD");
    let text_b = leaderboard::render(&rows_b, "BRIER LEADERBOARD");
    assert_eq!(
        text_a, text_b,
        "the leaderboard text must match byte for byte"
    );

    let lines_a = verdict::check_all(&rows_a);
    let lines_b = verdict::check_all(&rows_b);
    let verdict_text_a = verdict::render(&lines_a, "VERDICT");
    let verdict_text_b = verdict::render(&lines_b, "VERDICT");
    assert_eq!(
        verdict_text_a, verdict_text_b,
        "the verdict text must match byte for byte"
    );

    let standings_a = leaderboard::build_standings(&results_a, AggregationModel::Eject);
    let standings_b = leaderboard::build_standings(&results_b, AggregationModel::Eject);
    let eject_text_a = verdict::render(&verdict::check_all_eject(&standings_a), "VERDICT EJECT");
    let eject_text_b = verdict::render(&verdict::check_all_eject(&standings_b), "VERDICT EJECT");
    assert_eq!(
        eject_text_a, eject_text_b,
        "the eject verdict text must match byte for byte"
    );

    let flips_a = bootstrap::rank_flips(&results_a, &rows_a, 200, SEED);
    let flips_b = bootstrap::rank_flips(&results_b, &rows_b, 200, SEED);
    let flip_text_a = bootstrap::render_flips(&flips_a, "FLIPS");
    let flip_text_b = bootstrap::render_flips(&flips_b, "FLIPS");
    assert_eq!(
        flip_text_a, flip_text_b,
        "the bootstrap flip text must match byte for byte"
    );
}

#[test]
fn bootstrap_rank_flips_are_reproducible_for_a_fixed_seed() {
    let Some(mut instance) =
        load_instance_or_skip("bootstrap_rank_flips_are_reproducible_for_a_fixed_seed")
    else {
        return;
    };

    let (rows, results) = score_balanced(&mut instance, Metric::Brier);

    let flips_a = bootstrap::rank_flips(&results, &rows, 300, 777);
    let flips_b = bootstrap::rank_flips(&results, &rows, 300, 777);

    assert_eq!(flips_a.len(), flips_b.len());
    for (a, b) in flips_a.iter().zip(flips_b.iter()) {
        assert_eq!(a.upper, b.upper);
        assert_eq!(a.lower, b.lower);
        assert_eq!(a.upper_rank, b.upper_rank);
        assert_eq!(a.flip_count, b.flip_count);
        assert_eq!(a.flip_fraction, b.flip_fraction);
    }
}

#[test]
fn oracle_ranks_first_under_both_metrics() {
    let Some(mut instance) = load_instance_or_skip("oracle_ranks_first_under_both_metrics") else {
        return;
    };

    for metric in Metric::ALL {
        let (rows, _results) = score_balanced(&mut instance, metric);
        let top = rows
            .first()
            .expect("the leaderboard must have at least one row");
        assert_eq!(
            top.archetype,
            Archetype::Oracle,
            "metric {} did not rank oracle first",
            metric.name()
        );
    }
}
