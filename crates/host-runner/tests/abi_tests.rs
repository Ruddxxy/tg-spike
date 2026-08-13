//! These are integration tests for the host runner. They load a real
//! compiled `.wasm` module and check the same properties that the report
//! binary checks: the export surface, golden vectors, 1000-run bit
//! stability, and the malformed input matrix.
//!
//! These tests need a compiled `eval-script` `.wasm` file. On a clean
//! checkout, that file does not exist yet. When a test cannot find it, it
//! prints a clear message and returns early. It does not fail the test
//! suite for a missing build artefact.

use std::path::{Path, PathBuf};

use host_runner::cases;
use host_runner::checks;
use host_runner::instance::{AllocOutcome, ScriptInstance};

/// This gives the path to the `wasm32-unknown-unknown` release artefact.
fn unknown_unknown_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target/wasm32-unknown-unknown/release/eval_script.wasm")
}

/// This gives the path to the `wasm32-wasip1` release artefact.
fn wasip1_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target/wasm32-wasip1/release/eval_script.wasm")
}

/// This gives every `.wasm` path that exists on disk right now, plus a
/// label for each one. If none exist, it prints a message and gives an
/// empty list, so a caller can skip its test body.
fn available_wasm_paths() -> Vec<(&'static str, PathBuf)> {
    let mut found = Vec::new();
    for (label, path) in [
        ("wasm32-unknown-unknown", unknown_unknown_wasm_path()),
        ("wasm32-wasip1", wasip1_wasm_path()),
    ] {
        if path.exists() {
            found.push((label, path));
        } else {
            println!(
                "skip: no '.wasm' file at {} for target '{label}'. Build eval-script first.",
                path.display()
            );
        }
    }
    found
}

/// This macro-like helper skips a test body when no `.wasm` file exists,
/// instead of failing the test.
macro_rules! skip_if_none {
    ($paths:expr) => {
        if $paths.is_empty() {
            println!("skip: no eval-script '.wasm' file found. This test needs a build first.");
            return;
        }
    };
}

#[test]
fn export_surface_is_exactly_alloc_dealloc_and_rank_answer() {
    let paths = available_wasm_paths();
    skip_if_none!(paths);

    for (label, wasm_path) in &paths {
        let instance = ScriptInstance::load(wasm_path)
            .unwrap_or_else(|e| panic!("cannot load module for target '{label}': {e:?}"));
        let mut function_exports: Vec<&str> = instance
            .function_export_names()
            .iter()
            .map(String::as_str)
            .collect();
        function_exports.sort_unstable();
        assert_eq!(
            function_exports,
            vec!["alloc", "dealloc", "rank_answer"],
            "target '{label}': the published ABI is exactly alloc, dealloc, rank_answer"
        );
    }
}

#[test]
fn golden_vectors_match_by_bit_equality() {
    let paths = available_wasm_paths();
    skip_if_none!(paths);

    let golden_path = cases::golden_vectors_path();
    let vectors = cases::load_golden_vectors(&golden_path)
        .expect("golden_vectors.json must parse for this test to run");
    assert!(
        !vectors.is_empty(),
        "golden_vectors.json must have at least one vector"
    );

    for (label, wasm_path) in &paths {
        let mut instance = ScriptInstance::load(wasm_path)
            .unwrap_or_else(|e| panic!("cannot load module for target '{label}': {e:?}"));
        for vector in &vectors {
            let got = instance
                .rank_answer(
                    b"",
                    vector.ground_truth.as_bytes(),
                    vector.response.as_bytes(),
                )
                .unwrap_or_else(|e| {
                    panic!(
                        "call to 'rank_answer' failed for vector '{}' on '{label}': {e:?}",
                        vector.name
                    )
                });
            let expected_f32 = vector.expected as f32;
            assert_eq!(
                got.to_bits(),
                expected_f32.to_bits(),
                "vector '{}' on target '{label}': expected {} got {}",
                vector.name,
                expected_f32,
                got
            );
        }
    }
}

#[test]
fn rank_answer_is_bit_stable_across_1000_runs() {
    let paths = available_wasm_paths();
    skip_if_none!(paths);

    let gt = br#"{"label": 1}"#;
    let ma = br#"{"confidence": 0.75}"#;

    for (label, wasm_path) in &paths {
        let mut instance = ScriptInstance::load(wasm_path)
            .unwrap_or_else(|e| panic!("cannot load module for target '{label}': {e:?}"));
        let report = checks::check_rank_answer_repeat_stability(&mut instance, b"", gt, ma)
            .unwrap_or_else(|e| panic!("1000-run check failed on '{label}': {e:?}"));
        assert_eq!(
            report.distinct_count, 1,
            "target '{label}': expected one bit pattern across 1000 runs, saw {:?}",
            report.distinct_bits_hex
        );
    }
}

#[test]
fn malformed_inputs_all_return_worst_score_and_never_trap() {
    let paths = available_wasm_paths();
    skip_if_none!(paths);

    let matrix = cases::malformed_cases();
    let worst_bits = 0.0_f32.to_bits();

    for (label, wasm_path) in &paths {
        let mut instance = ScriptInstance::load(wasm_path)
            .unwrap_or_else(|e| panic!("cannot load module for target '{label}': {e:?}"));
        for case in &matrix {
            let result = instance.rank_answer(b"", &case.ground_truth, &case.response);
            match result {
                // No case in the current matrix sets `expect_worst_score`
                // to false. This arm stays here in case a future case
                // checks cost and safety on a large but well-formed
                // payload without a fixed score.
                Ok(_) if !case.expect_worst_score => {}
                Ok(value) => assert_eq!(
                    value.to_bits(),
                    worst_bits,
                    "target '{label}', case {} ('{}'): expected 0.0, got {value}",
                    case.id,
                    case.name
                ),
                Err(e) => panic!(
                    "target '{label}', case {} ('{}') trapped instead of returning 0.0: {e:?}",
                    case.id, case.name
                ),
            }
        }
    }
}

/// This is a smoke test for the "fresh vs reused" property, run at a
/// small scale so the test suite stays fast.
#[test]
fn fresh_instance_matches_reused_instance() {
    let paths = available_wasm_paths();
    skip_if_none!(paths);

    let gt = br#"{"label": 0}"#;
    let ma = br#"{"confidence": 0.25}"#;

    for (label, wasm_path) in &paths {
        let mut instance = ScriptInstance::load(wasm_path)
            .unwrap_or_else(|e| panic!("cannot load module for target '{label}': {e:?}"));
        // Warm the reused instance up with a few unrelated calls first,
        // so this test can catch state that leaks between calls.
        for _ in 0..5 {
            instance
                .rank_answer(b"", br#"{"label": 1}"#, br#"{"confidence": 0.9}"#)
                .unwrap_or_else(|e| panic!("warm-up call failed on '{label}': {e:?}"));
        }
        let report = checks::check_fresh_vs_reused(wasm_path.as_path(), &mut instance, b"", gt, ma)
            .unwrap_or_else(|e| panic!("fresh-vs-reused check failed on '{label}': {e:?}"));
        assert!(
            report.pass,
            "target '{label}': fresh {} != reused {}",
            report.fresh_bits_hex, report.reused_bits_hex
        );
    }
}

/// This checks that the helper for finding `golden_vectors.json` points at
/// a real file, independent of whether any `.wasm` module exists yet.
#[test]
fn golden_vectors_file_is_readable() {
    let path: PathBuf = cases::golden_vectors_path();
    assert!(
        Path::new(&path).exists(),
        "golden_vectors.json must exist at {}",
        path.display()
    );
    let vectors = cases::load_golden_vectors(&path).expect("golden_vectors.json must parse");
    assert!(
        !vectors.is_empty(),
        "golden_vectors.json must have at least one vector"
    );
}

/// This is the test that proves the fix for the `alloc` size cap.
///
/// It records the module linear memory size, in pages, before and
/// after a rejected oversized `alloc` call, and checks the size did
/// not change. A rejected `alloc` call must never grow memory. A
/// timing gain alone does not prove that; only this page count does.
#[test]
fn rejected_oversized_alloc_does_not_grow_linear_memory() {
    let paths = available_wasm_paths();
    skip_if_none!(paths);

    let oversized = vec![b'x'; (cases::MAX_INPUT_BYTES + 1) as usize];

    for (label, wasm_path) in &paths {
        let mut instance = ScriptInstance::load(wasm_path)
            .unwrap_or_else(|e| panic!("cannot load module for target '{label}': {e:?}"));

        let pages_before = instance.memory_size_pages();
        let outcome = instance
            .write_bytes(&oversized)
            .unwrap_or_else(|e| panic!("'alloc' call failed on '{label}': {e:?}"));
        let pages_after = instance.memory_size_pages();

        println!("target '{label}': memory pages before = {pages_before}, after = {pages_after}");

        assert_eq!(
            outcome,
            AllocOutcome::Rejected,
            "target '{label}': an oversized request of {} bytes must be rejected, cap is {} bytes",
            oversized.len(),
            cases::MAX_INPUT_BYTES
        );
        assert_eq!(
            pages_before, pages_after,
            "target '{label}': a rejected 'alloc' call must not grow linear memory, \
             saw {pages_before} pages before and {pages_after} pages after"
        );
    }
}

/// This proves a rejected oversized `alloc` call leaves the module in a
/// usable state.
///
/// After the rejection, a normal golden-vector alloc/write/call/free
/// cycle must still give the exact expected score, by bit equality.
/// This rules out a corrupted allocator or leftover state from the
/// rejected request.
#[test]
fn valid_cycle_after_a_rejected_alloc_still_gives_the_right_golden_vector_score() {
    let paths = available_wasm_paths();
    skip_if_none!(paths);

    let golden_path = cases::golden_vectors_path();
    let vectors = cases::load_golden_vectors(&golden_path)
        .expect("golden_vectors.json must parse for this test to run");
    let vector = vectors
        .first()
        .expect("golden_vectors.json must have at least one vector");

    let oversized = vec![b'x'; (cases::MAX_INPUT_BYTES + 1) as usize];

    for (label, wasm_path) in &paths {
        let mut instance = ScriptInstance::load(wasm_path)
            .unwrap_or_else(|e| panic!("cannot load module for target '{label}': {e:?}"));

        // Trigger a rejected allocation first.
        let outcome = instance
            .write_bytes(&oversized)
            .unwrap_or_else(|e| panic!("'alloc' call failed on '{label}': {e:?}"));
        assert_eq!(
            outcome,
            AllocOutcome::Rejected,
            "target '{label}': setup step must reject this oversized request"
        );

        // A normal score cycle right after the rejection must still work
        // and must still match the golden vector by bit equality.
        let got = instance
            .rank_answer(
                b"",
                vector.ground_truth.as_bytes(),
                vector.response.as_bytes(),
            )
            .unwrap_or_else(|e| {
                panic!("call to 'rank_answer' failed on '{label}' after a rejected alloc: {e:?}")
            });
        let expected_f32 = vector.expected as f32;
        assert_eq!(
            got.to_bits(),
            expected_f32.to_bits(),
            "target '{label}': after a rejected alloc, vector '{}' expected {} got {}",
            vector.name,
            expected_f32,
            got
        );
    }
}

/// This proves `alloc` still succeeds through the real module when the
/// request sits exactly at `MAX_INPUT_BYTES`. The cap rejects a size
/// over the cap, not a size at the cap.
#[test]
fn alloc_at_exactly_the_cap_succeeds_through_the_real_module() {
    let paths = available_wasm_paths();
    skip_if_none!(paths);

    let at_cap = vec![b'x'; cases::MAX_INPUT_BYTES as usize];

    for (label, wasm_path) in &paths {
        let mut instance = ScriptInstance::load(wasm_path)
            .unwrap_or_else(|e| panic!("cannot load module for target '{label}': {e:?}"));

        let outcome = instance
            .write_bytes(&at_cap)
            .unwrap_or_else(|e| panic!("'alloc' call failed on '{label}': {e:?}"));
        match outcome {
            AllocOutcome::Granted(ptr, len) => {
                assert_eq!(len, cases::MAX_INPUT_BYTES, "target '{label}'");
                instance
                    .free(ptr, len)
                    .unwrap_or_else(|e| panic!("'dealloc' call failed on '{label}': {e:?}"));
            }
            AllocOutcome::Rejected => panic!(
                "target '{label}': a request of exactly {} bytes, the cap, must be granted",
                at_cap.len()
            ),
        }
    }
}
