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
//! This binary prints a plain text report to stdout. `println!` is the
//! right tool here: this binary IS the report, not a debug trace left in
//! committed code.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use host_runner::cases::{self, MalformedCase};
use host_runner::checks;
use host_runner::instance::ScriptInstance;

/// This is the default `.wasm` path, used when no path is given on the
/// command line.
const DEFAULT_WASM_PATH: &str = "target/wasm32-unknown-unknown/release/eval_script.wasm";

fn main() {
    let wasm_path = resolve_wasm_path();
    match run_report(&wasm_path) {
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

/// This runs every report section in order. It returns true if every
/// section passed.
fn run_report(wasm_path: &Path) -> Result<bool> {
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

    let gt_bytes = golden_outcome.first_ground_truth;
    let resp_bytes = golden_outcome.first_response;

    let repeat_pass = run_repeat_stability(&mut instance, &gt_bytes, &resp_bytes)?;
    section_pass.push(("1000-run stability", repeat_pass));

    let fresh_pass = run_fresh_vs_reused(wasm_path, &mut instance, &gt_bytes, &resp_bytes)?;
    section_pass.push(("fresh vs reused instance", fresh_pass));

    let order_pass = run_order_invariance(&mut instance)?;
    section_pass.push(("batch order invariance", order_pass));

    let malformed_pass = run_malformed_matrix(&mut instance)?;
    section_pass.push(("malformed input matrix", malformed_pass));

    print_summary(&section_pass);

    Ok(section_pass.iter().all(|(_, pass)| *pass))
}

/// This prints the module info section: path, size, instantiation path,
/// and export names.
fn print_module_info(wasm_path: &Path, instance: &ScriptInstance) -> bool {
    println!();
    println!("--- 1. Module info ---");
    println!("path: {}", wasm_path.display());
    println!("file size: {} bytes", instance.file_size_bytes());
    println!("instantiation path: {}", instance.instantiation_path());
    println!("export names:");
    for name in instance.export_names() {
        println!("  - {name}");
    }
    true
}

/// This is the result of the golden vector section.
struct GoldenVectorsOutcome {
    /// True if every vector matched its expected score by bit equality.
    all_match: bool,
    /// The ground truth bytes of the first vector. Later sections reuse
    /// this as one fixed, known-good input.
    first_ground_truth: Vec<u8>,
    /// The response bytes of the first vector. Later sections reuse this
    /// as one fixed, known-good input.
    first_response: Vec<u8>,
}

/// This runs the golden vector table and prints a row for each vector.
///
/// It returns whether every vector matched by bit equality, plus the raw
/// bytes of the first vector, for reuse in later sections.
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
        "{:<28} {:>12} {:>12} {:>6}",
        "name", "expected", "got", "match"
    );
    let mut all_match = true;
    let mut first_bytes: Option<(Vec<u8>, Vec<u8>)> = None;
    for vector in &vectors {
        let gt_bytes = vector.ground_truth.as_bytes();
        let resp_bytes = vector.response.as_bytes();
        let got = instance.score(gt_bytes, resp_bytes).with_context(|| {
            format!("call to 'score' failed for golden vector '{}'", vector.name)
        })?;
        let bits_match = got.to_bits() == vector.expected.to_bits();
        all_match &= bits_match;
        println!(
            "{:<28} {:>12.6} {:>12.6} {:>6}",
            vector.name,
            vector.expected,
            got,
            if bits_match { "yes" } else { "NO" }
        );
        if first_bytes.is_none() {
            first_bytes = Some((gt_bytes.to_vec(), resp_bytes.to_vec()));
        }
    }
    println!("all vectors match by bit equality: {all_match}");

    let (first_ground_truth, first_response) =
        first_bytes.expect("the vectors list was checked to be non-empty above");
    Ok(GoldenVectorsOutcome {
        all_match,
        first_ground_truth,
        first_response,
    })
}

/// This runs the 1000-call stability check for `score` and
/// `score_log_loss`, on one fixed input, and prints the result.
fn run_repeat_stability(instance: &mut ScriptInstance, gt: &[u8], resp: &[u8]) -> Result<bool> {
    println!();
    println!("--- 3. Determinism: 1000 identical runs ---");

    let score_report = checks::check_score_repeat_stability(instance, gt, resp)
        .context("the 1000-run stability check for 'score' failed to complete")?;
    println!(
        "score: {} runs, bit pattern {}, distinct values {}",
        score_report.run_count, score_report.bits_hex, score_report.distinct_count
    );
    if !score_report.pass {
        println!(
            "score: distinct bit patterns seen: {:?}",
            score_report.distinct_bits_hex
        );
    }

    let log_loss_report = checks::check_score_log_loss_repeat_stability(instance, gt, resp)
        .context("the 1000-run stability check for 'score_log_loss' failed to complete")?;
    println!(
        "score_log_loss: {} runs, bit pattern {}, distinct values {}",
        log_loss_report.run_count, log_loss_report.bits_hex, log_loss_report.distinct_count
    );
    if !log_loss_report.pass {
        println!(
            "score_log_loss: distinct bit patterns seen: {:?}",
            log_loss_report.distinct_bits_hex
        );
    }

    Ok(score_report.pass && log_loss_report.pass)
}

/// This runs the fresh-instance-vs-reused-instance check and prints the
/// result.
fn run_fresh_vs_reused(
    wasm_path: &Path,
    instance: &mut ScriptInstance,
    gt: &[u8],
    resp: &[u8],
) -> Result<bool> {
    println!();
    println!("--- 4. Determinism: fresh instance vs reused instance ---");
    let report = checks::check_fresh_vs_reused(wasm_path, instance, gt, resp)
        .context("the fresh-vs-reused check failed to complete")?;
    println!("fresh instance bit pattern:  {}", report.fresh_bits_hex);
    println!("reused instance bit pattern: {}", report.reused_bits_hex);
    println!("match: {}", report.pass);
    Ok(report.pass)
}

/// This runs the batch order invariance check and prints every ordering
/// tried, plus its result bit pattern.
fn run_order_invariance(instance: &mut ScriptInstance) -> Result<bool> {
    println!();
    println!("--- 5. Order invariance of a batch ---");
    let (results, pass) = checks::check_batch_order_invariance(instance)
        .context("the batch order invariance check failed to complete")?;
    for result in &results {
        println!("order '{}': bit pattern {}", result.name, result.bits_hex);
    }
    println!("all orders match by bit equality: {pass}");
    Ok(pass)
}

/// This runs every case in the malformed input matrix and prints a table
/// row for each one. A trap counts as a failure, but it does not stop the
/// run.
fn run_malformed_matrix(instance: &mut ScriptInstance) -> Result<bool> {
    println!();
    println!("--- 6. Malformed input matrix ---");
    let matrix = cases::malformed_cases();

    println!(
        "{:>3} {:<45} {:<50} {:<50} {:>10} {:>10} {:>8} {:>4}",
        "#", "case", "ground_truth", "response", "expected", "got", "defence", "ok"
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
    println!("  score  - 'alloc' let the input through. The scoring function");
    println!("           itself found the problem, for example bad JSON.");
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

/// This runs one malformed case through `score`. It catches a trap as a
/// host-side error, instead of letting it stop the whole run.
///
/// It returns whether the case passed, the text to show in the "got"
/// column, an optional timing note for the huge-payload cases, and the
/// "defence" tag for that case: `"alloc"` when the module's `alloc`
/// cap rejected the input, `"score"` when the scoring function itself
/// produced the worst score, or `"-"` when the case did not produce a
/// worst score at all.
fn run_one_malformed_case(
    instance: &mut ScriptInstance,
    case: &MalformedCase,
) -> (bool, String, Option<String>, &'static str) {
    let is_huge = case.ground_truth.len() > 1_000_000 || case.response.len() > 1_000_000;
    let start = Instant::now();
    let result = instance.score_outcome(&case.ground_truth, &case.response);
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
            } else if value.to_bits() == 0.0_f64.to_bits() {
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
