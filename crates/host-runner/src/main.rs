//! This is the host runner binary. It stands in for a Telegraph
//! validator. It loads a compiled `eval-script` WASM module and checks
//! that the module gives correct, stable, and safe scores.
//!
//! Run it like this:
//! ```text
//! cargo run -p host-runner --release -- path/to/eval_script.wasm
//! ```
//! If no path is given, it uses the default `wasm32-unknown-unknown`
//! release path under the workspace `target` directory.
//!
//! A second, optional argument names the wazero side golden result
//! file, for the cross host bit equality section. It defaults to
//! [`host_runner::cross_host::DEFAULT_WAZERO_GOLDEN_PATH`]:
//! ```text
//! cargo run -p host-runner --release -- path/to/eval_script.wasm path/to/golden-f32-wazero.json
//! ```
//!
//! This binary prints a plain text report to stdout. `println!` is the
//! right tool here: this binary IS the report, not a debug trace left in
//! committed code.
//!
//! ## Run order
//!
//! The cross host section (section 6) needs a wazero side golden result
//! file. Build that file first, with the Go `wazero-runner` tool, in
//! golden mode, against the same `.wasm` file this binary loads. See
//! the workspace `README.md` for the exact command line.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use host_runner::cases::{self, MalformedCase};
use host_runner::checks;
use host_runner::cross_host::{self, CrossHostOutcome};
use host_runner::golden::{self, GoldenResult};
use host_runner::instance::ScriptInstance;

/// This is the default `.wasm` path, used when no path is given on the
/// command line.
const DEFAULT_WASM_PATH: &str = "target/wasm32-unknown-unknown/release/eval_script.wasm";

/// This is the path this binary writes its own golden vector results
/// to, in the shape [`host_runner::golden::GoldenOutput`] defines.
const WASMTIME_GOLDEN_OUT_PATH: &str = "target/golden-f32-wasmtime.json";

fn main() {
    let wasm_path = resolve_wasm_path();
    let wazero_golden_path = resolve_wazero_golden_path();
    match run_report(&wasm_path, &wazero_golden_path) {
        Ok(all_passed) => {
            if all_passed {
                std::process::exit(0);
            } else {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("host-runner stopped with an error: {e:?}");
            std::process::exit(1);
        }
    }
}

/// This reads the `.wasm` path from `argv[1]`, or uses the default path.
fn resolve_wasm_path() -> PathBuf {
    match std::env::args().nth(1) {
        Some(arg) => PathBuf::from(arg),
        None => PathBuf::from(DEFAULT_WASM_PATH),
    }
}

/// This reads the wazero golden result path from `argv[2]`, or uses
/// [`cross_host::DEFAULT_WAZERO_GOLDEN_PATH`].
fn resolve_wazero_golden_path() -> PathBuf {
    match std::env::args().nth(2) {
        Some(arg) => PathBuf::from(arg),
        None => PathBuf::from(cross_host::DEFAULT_WAZERO_GOLDEN_PATH),
    }
}

/// This runs every report section in order. It returns true if every
/// section passed.
fn run_report(wasm_path: &Path, wazero_golden_path: &Path) -> Result<bool> {
    println!("======================================================");
    println!(" Telegraph Track 2 host runner");
    println!("======================================================");
    println!();
    println!("Score direction: a HIGH score is good. The worst score is 0.0.");
    println!("The best score is 1.0. Source: Telegraph whitepaper v1.0,");
    println!("section 7.4 (the router sends traffic in proportion to the");
    println!("score) and section 4.3 (a score above 0.70 is a possible");
    println!("overscore).");

    let mut instance = ScriptInstance::load(wasm_path)
        .with_context(|| format!("cannot load module at {}", wasm_path.display()))?;

    let mut section_pass = Vec::new();

    section_pass.push(("module info", print_module_info(wasm_path, &instance)));

    let golden_outcome = run_golden_vectors(&mut instance)?;
    section_pass.push(("golden vectors", golden_outcome.all_match));

    write_wasmtime_golden_file(wasm_path, &instance, &golden_outcome.results)?;

    let gt_bytes = &golden_outcome.first_ground_truth;
    let ma_bytes = &golden_outcome.first_miner_answer;

    let repeat_pass = run_repeat_stability(&mut instance, gt_bytes, ma_bytes)?;
    section_pass.push(("1000-run stability", repeat_pass));

    let fresh_pass = run_fresh_vs_reused(wasm_path, &mut instance, gt_bytes, ma_bytes)?;
    section_pass.push(("fresh vs reused instance", fresh_pass));

    let malformed_pass = run_malformed_matrix(&mut instance)?;
    section_pass.push(("malformed input matrix", malformed_pass));

    let cross_host_pass = run_cross_host(
        wasm_path,
        &instance,
        &golden_outcome.results,
        wazero_golden_path,
    );
    section_pass.push(("wasmtime vs wazero cross host", cross_host_pass));

    print_summary(&section_pass);

    Ok(section_pass.iter().all(|(_, pass)| *pass))
}

/// This prints the module info section: path, size, hash, instantiation
/// path, and export names. It asserts the export surface: PASS only if
/// the function exports are exactly `alloc`, `dealloc`, and
/// `rank_answer`.
fn print_module_info(wasm_path: &Path, instance: &ScriptInstance) -> bool {
    println!();
    println!("--- 1. Module info ---");
    println!("path: {}", wasm_path.display());
    println!("file size: {} bytes", instance.file_size_bytes());
    println!("sha256: {}", instance.wasm_sha256());
    println!("instantiation path: {}", instance.instantiation_path());
    println!("export names:");
    for name in instance.export_names() {
        println!("  - {name}");
    }
    println!();
    println!("note: 'memory' is a required export. The host reads and writes");
    println!("module linear memory through it.");
    println!("note: '__data_end' and '__heap_base' are globals, not functions.");
    println!("They are rust-lld defaults for a wasm32-unknown-unknown build.");
    println!("The protocol's own reference module also exports these two");
    println!("globals.");

    let mut function_exports: Vec<&str> = instance
        .function_export_names()
        .iter()
        .map(String::as_str)
        .collect();
    function_exports.sort_unstable();
    let expected = ["alloc", "dealloc", "rank_answer"];
    let pass = function_exports == expected;
    println!();
    println!("function exports found: {}", function_exports.join(", "));
    println!("function exports expected: {}", expected.join(", "));
    println!("function export surface matches the published ABI exactly: {pass}");
    pass
}

/// This is the result of the golden vector section.
struct GoldenVectorsOutcome {
    /// True if every vector matched its expected score by bit equality.
    all_match: bool,
    /// The ground truth bytes of the first vector. Later sections reuse
    /// this as one fixed, known-good input.
    first_ground_truth: Vec<u8>,
    /// The miner answer (response) bytes of the first vector. Later
    /// sections reuse this as one fixed, known-good input.
    first_miner_answer: Vec<u8>,
    /// This run's own `rank_answer` result for every golden vector, in
    /// the shared golden JSON shape. `write_wasmtime_golden_file` writes
    /// this list to disk. `run_cross_host` compares it against the
    /// wazero side result file.
    results: Vec<GoldenResult>,
}

/// This runs the golden vector table and prints a row for each vector.
///
/// Every vector uses an empty question: `rank_answer(question="", ...)`.
/// This matches the fixed convention the `wazero-runner` golden mode
/// uses, so the two hosts' results can compare directly in section 6.
/// `golden_vectors.json` stores `expected` as `f64`. This function
/// compares `got.to_bits() == (vector.expected as f32).to_bits()`,
/// because `rank_answer` returns `f32`.
///
/// It returns whether every vector matched by bit equality, the raw
/// ground truth and miner answer bytes of the first vector for reuse in
/// later sections, and this run's full result list for the wasmtime
/// golden file and the cross host check.
fn run_golden_vectors(instance: &mut ScriptInstance) -> Result<GoldenVectorsOutcome> {
    println!();
    println!("--- 2. Golden vectors ---");
    let path = cases::golden_vectors_path();
    let vectors = cases::load_golden_vectors(&path)
        .with_context(|| format!("cannot load golden vectors from {}", path.display()))?;

    if vectors.is_empty() {
        anyhow::bail!(
            "the golden vectors file at {} has no vectors",
            path.display()
        );
    }

    println!(
        "question: taken from each vector, the same bytes the wazero runner sends.\n\
         One vector carries a junk question on purpose."
    );
    println!(
        "{:<28} {:>12} {:>12} {:>12} {:>6}",
        "name", "expected", "got", "got bits", "match"
    );
    let mut all_match = true;
    let mut first_bytes: Option<(Vec<u8>, Vec<u8>)> = None;
    let mut results = Vec::with_capacity(vectors.len());
    for vector in &vectors {
        let question_bytes = vector.question.as_bytes();
        let gt_bytes = vector.ground_truth.as_bytes();
        let ma_bytes = vector.miner_answer.as_bytes();
        let got = instance
            .rank_answer(question_bytes, gt_bytes, ma_bytes)
            .with_context(|| {
                format!(
                    "call to 'rank_answer' failed for golden vector '{}'",
                    vector.name
                )
            })?;
        let expected_f32 = vector.expected as f32;
        let bits_match = got.to_bits() == expected_f32.to_bits();
        all_match &= bits_match;
        println!(
            "{:<28} {:>12.6} {:>12.6} {:>12} {:>6}",
            vector.name,
            expected_f32,
            got,
            format!("0x{:08x}", got.to_bits()),
            if bits_match { "yes" } else { "NO" }
        );
        results.push(golden::golden_result(&vector.name, got));
        if first_bytes.is_none() {
            first_bytes = Some((gt_bytes.to_vec(), ma_bytes.to_vec()));
        }
    }
    println!("all vectors match by bit equality: {all_match}");

    let (first_ground_truth, first_miner_answer) =
        first_bytes.expect("the vectors list was checked to be non-empty above");
    Ok(GoldenVectorsOutcome {
        all_match,
        first_ground_truth,
        first_miner_answer,
        results,
    })
}

/// This writes this run's golden vector results to
/// [`WASMTIME_GOLDEN_OUT_PATH`], in the shape [`golden::GoldenOutput`]
/// defines, with `"runner": "wasmtime"`.
///
/// A later run of the Go `wazero-runner` tool, in golden mode, on the
/// same `.wasm` file, writes a matching file for the wazero side. See
/// `run_cross_host` for the check that compares the two.
fn write_wasmtime_golden_file(
    wasm_path: &Path,
    instance: &ScriptInstance,
    results: &[GoldenResult],
) -> Result<()> {
    let out_path = Path::new(WASMTIME_GOLDEN_OUT_PATH);
    golden::write_wasmtime_golden(
        out_path,
        &wasm_path.display().to_string(),
        instance.wasm_sha256(),
        results,
    )
    .with_context(|| {
        format!(
            "cannot write wasmtime golden result file to {}",
            out_path.display()
        )
    })?;
    println!();
    println!(
        "wrote {} wasmtime golden vector results to {}",
        results.len(),
        out_path.display()
    );
    Ok(())
}

/// This runs the 1000-call stability check for `rank_answer`, on one
/// fixed input, and prints the result.
fn run_repeat_stability(instance: &mut ScriptInstance, gt: &[u8], ma: &[u8]) -> Result<bool> {
    println!();
    println!("--- 3. Determinism: 1000 identical runs ---");

    let rank_report = checks::check_rank_answer_repeat_stability(instance, b"", gt, ma)
        .context("the 1000-run stability check for 'rank_answer' failed to complete")?;
    println!(
        "rank_answer (wasm): {} runs, bit pattern {}, distinct values {}",
        rank_report.run_count, rank_report.bits_hex, rank_report.distinct_count
    );
    if !rank_report.pass {
        println!(
            "rank_answer: distinct bit patterns seen: {:?}",
            rank_report.distinct_bits_hex
        );
    }

    Ok(rank_report.pass)
}

/// This runs the fresh-instance-vs-reused-instance check and prints the
/// result.
fn run_fresh_vs_reused(
    wasm_path: &Path,
    instance: &mut ScriptInstance,
    gt: &[u8],
    ma: &[u8],
) -> Result<bool> {
    println!();
    println!("--- 4. Determinism: fresh instance vs reused instance ---");
    let report = checks::check_fresh_vs_reused(wasm_path, instance, b"", gt, ma)
        .context("the fresh-vs-reused check failed to complete")?;
    println!("fresh instance bit pattern:  {}", report.fresh_bits_hex);
    println!("reused instance bit pattern: {}", report.reused_bits_hex);
    println!("match: {}", report.pass);
    Ok(report.pass)
}

/// This runs every case in the malformed input matrix and prints a table
/// row for each one. A trap counts as a failure, but it does not stop the
/// run. Every case uses an empty question, matching section 2.
fn run_malformed_matrix(instance: &mut ScriptInstance) -> Result<bool> {
    println!();
    println!("--- 5. Malformed input matrix ---");
    let matrix = cases::malformed_cases();

    println!(
        "{:>3} {:<45} {:<50} {:<50} {:>10} {:>10} {:>8} {:>4}",
        "#", "case", "ground_truth", "miner answer", "expected", "got", "defence", "ok"
    );
    println!(
        "note: cases 23 and 24 are far over the {} byte input cap.",
        cases::MAX_INPUT_BYTES
    );
    println!("The cap rejects them before the host reads any byte from memory.");
    println!("Case 25 sits exactly at the cap. The cap lets it through; it fails");
    println!("to parse instead, so it still returns 0.0. Every case in this");
    println!("matrix must return exactly 0.0, the worst score.");
    println!("The 'defence' column shows which check produced the 0.0 score:");
    println!("  alloc  - the module's 'alloc' export rejected the size. The host");
    println!("           never wrote a byte into linear memory for that case.");
    println!("  score  - 'rank_answer' let the input through. It found the");
    println!("           problem itself, for example bad JSON.");
    println!("  -      - the case did not get a worst score at all; the module");
    println!("           returned a real score.");

    let mut all_ok = true;
    for case in &matrix {
        let (row_ok, got_text, elapsed_note, defence) = run_one_malformed_case(instance, case);
        all_ok &= row_ok;
        let gt_display = cases::truncate_for_display(&case.ground_truth, 40);
        let resp_display = cases::truncate_for_display(&case.response, 40);
        let expected_text = if case.expect_worst_score {
            "0.0"
        } else {
            "(real score)"
        };
        println!(
            "{:>3} {:<45} {:<50} {:<50} {:>10} {:>10} {:>8} {:>4}",
            case.id,
            case.name,
            gt_display,
            resp_display,
            expected_text,
            got_text,
            defence,
            if row_ok { "yes" } else { "NO" }
        );
        if let Some(note) = elapsed_note {
            println!("    ({note})");
        }
    }
    println!("every case matches its expected outcome and does not trap: {all_ok}");
    Ok(all_ok)
}

/// This runs one malformed case through `rank_answer`, with an empty
/// question. It catches a trap as a host-side error, instead of letting
/// it stop the whole run.
///
/// It returns whether the case passed, the text to show in the "got"
/// column, an optional timing note for the huge-payload cases, and the
/// "defence" tag for that case: `"alloc"` when the module's `alloc`
/// cap rejected the input, `"score"` when `rank_answer` itself
/// produced the worst score, or `"-"` when the case did not produce a
/// worst score at all.
fn run_one_malformed_case(
    instance: &mut ScriptInstance,
    case: &MalformedCase,
) -> (bool, String, Option<String>, &'static str) {
    let is_huge = case.ground_truth.len() > 1_000_000 || case.response.len() > 1_000_000;
    let start = Instant::now();
    let result = instance.rank_answer_outcome(b"", &case.ground_truth, &case.response);
    let elapsed = start.elapsed();

    let elapsed_note = if is_huge {
        Some(format!("wall time: {elapsed:.2?}"))
    } else {
        None
    };

    match result {
        Ok(outcome) => {
            let defence = if outcome.alloc_rejected {
                "alloc"
            } else if case.expect_worst_score {
                "score"
            } else {
                "-"
            };
            let value = outcome.value;
            if !case.expect_worst_score {
                // No case in the current matrix sets `expect_worst_score`
                // to false. This branch stays here so a future case can
                // check cost and safety on a large but well-formed
                // payload without a fixed score, the way case 23 did
                // before the input cap.
                (true, format!("{value}"), elapsed_note, defence)
            } else if value.to_bits() == 0.0_f32.to_bits() {
                (true, format!("{value}"), elapsed_note, defence)
            } else {
                (
                    false,
                    format!("{value} (expected 0.0)"),
                    elapsed_note,
                    defence,
                )
            }
        }
        Err(e) => (false, format!("TRAP ({e})"), elapsed_note, "-"),
    }
}

/// This runs the wasmtime vs wazero cross host bit equality check and
/// prints the result.
///
/// A wasmtime/wazero disagreement on the same golden vector is a
/// consensus relevant defect: the Telegraph network can run either
/// engine underneath its wasm host, and two honest validators on
/// different engines must never produce different Local Scores for the
/// same input. See [`cross_host`] for the full reasoning and for the
/// three failure cases this prints.
fn run_cross_host(
    wasm_path: &Path,
    instance: &ScriptInstance,
    wasmtime_results: &[GoldenResult],
    wazero_path: &Path,
) -> bool {
    println!();
    println!("--- 6. wasmtime vs wazero cross host bit equality ---");
    println!("A wasmtime/wazero disagreement is a consensus relevant defect: the");
    println!("network can run either engine, and two honest validators on");
    println!("different engines must never disagree on a score.");
    println!("wazero result file: {}", wazero_path.display());

    let outcome = cross_host::compare_cross_host(
        wasmtime_results,
        instance.wasm_sha256(),
        &wasm_path.display().to_string(),
        wazero_path,
    );

    match outcome {
        CrossHostOutcome::MissingFile { command } => {
            println!(
                "FAIL: no usable wazero golden result file at {}.",
                wazero_path.display()
            );
            println!("Missing cross host evidence must not silently pass.");
            println!("Run this command to produce it:");
            println!("  {command}");
            false
        }
        CrossHostOutcome::Stale {
            wasmtime_sha256,
            wazero_sha256,
        } => {
            println!("FAIL: the wazero golden result file is STALE.");
            println!("wasmtime side .wasm sha256: {wasmtime_sha256}");
            println!("wazero file's recorded sha256: {wazero_sha256}");
            println!("These hashes must match. A stale file must not fake agreement.");
            println!("Rebuild the wazero file against the same .wasm this run loaded.");
            false
        }
        CrossHostOutcome::Compared {
            results,
            extra_wazero_names,
            pass,
        } => {
            println!(
                "{:<28} {:>14} {:>14} {:>6}",
                "name", "wasmtime bits", "wazero bits", "match"
            );
            for r in &results {
                println!(
                    "{:<28} {:>14} {:>14} {:>6}",
                    r.name,
                    r.wasmtime_bits_hex,
                    r.wazero_bits_hex,
                    if r.bit_match { "yes" } else { "NO" }
                );
                if !r.bit_match {
                    println!(
                        "  DISAGREEMENT: wasmtime value {} bits {}, wazero value {} bits {}",
                        r.wasmtime_value, r.wasmtime_bits_hex, r.wazero_value, r.wazero_bits_hex
                    );
                    println!("  a wasmtime/wazero disagreement is a consensus relevant defect.");
                }
            }
            println!("vectors compared: {}", results.len());
            if !extra_wazero_names.is_empty() {
                println!(
                    "FAIL: the wazero file names {} vector(s) this run did not score: {}",
                    extra_wazero_names.len(),
                    extra_wazero_names.join(", ")
                );
                println!("  the two files must name the exact same set of vectors. An extra name");
                println!("  means the two sides did not score the same golden vector file.");
            }
            println!("all vectors bit identical across wasmtime and wazero: {pass}");
            pass
        }
    }
}

/// This prints the final pass/fail summary for every section.
fn print_summary(section_pass: &[(&str, bool)]) {
    println!();
    println!("--- 7. Summary ---");
    for (name, pass) in section_pass {
        println!("{}: {}", if *pass { "PASS" } else { "FAIL" }, name);
    }
    let overall = section_pass.iter().all(|(_, pass)| *pass);
    println!();
    println!("overall verdict: {}", if overall { "PASS" } else { "FAIL" });
}
