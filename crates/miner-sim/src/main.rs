//! This binary is the miner simulator report.
//!
//! The binary makes synthetic miners with a known true quality. It
//! scores every miner through the compiled `eval-script` WASM module,
//! with the Brier metric, the one scoring rule the published ABI
//! supports. It checks that the leaderboard order matches the known
//! quality order. It prints one report to standard output.
//!
//! `println!` is correct in this file. This binary IS the report; the
//! printed text is the deliverable, not debug output.
//!
//! The binary never tunes a threshold to turn a failed check into a
//! passed one. A failed check is a real finding about the scoring
//! rule under test.
//!
//! # Score direction
//!
//! A HIGH score is good. Read the note on score direction in
//! `types.rs`. Every leaderboard and every standings table in this
//! report is "higher is better".
//!
//! # Two aggregation models
//!
//! This report prints standings under both aggregation models, for
//! every data set:
//!
//! - `score_and_keep` keeps every miner ranked, even a miner that never
//!   answered or sent bad text. This is NOT the protocol rule. The
//!   report keeps it only so a reader can see how much the aggregation
//!   layer changes the outcome.
//! - `eject` removes a miner from the pool at its first failure. This
//!   IS the protocol rule. Read the whitepaper v1.0, section 5.1.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use host_runner::instance::ScriptInstance;

use miner_sim::types::{
    AggregationModel, Archetype, DatasetShape, Metric, ResponseSeed, VerdictLine,
};
use miner_sim::{archetype, bootstrap, dataset, leaderboard, scoring, verdict};

/// The item count of each main data set.
const ITEMS: usize = 2000;
/// The base seed of the whole report. Every data set and every
/// bootstrap round derives its own seed from this value, so the whole
/// report stays fixed for this one constant.
const SEED: u64 = 0xC0FFEE;
/// The count of bootstrap resamples for the rank flip tables and the
/// gap confidence ranges.
const RESAMPLES: usize = 1000;
/// The sample size ladder for the top two gap report.
const GAP_LADDER: [usize; 8] = [50, 100, 200, 400, 800, 1600, 3200, 6400];

/// This is a record of one invariant that failed, kept for the final
/// summary.
struct FailedInvariant {
    /// The name of the data set shape where the check ran.
    shape_name: &'static str,
    /// The name of the metric where the check ran.
    metric_name: &'static str,
    /// The name of the aggregation model where the check ran:
    /// `score_and_keep` or `eject`.
    model_name: &'static str,
    /// The verdict line that failed.
    line: VerdictLine,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// This function runs the whole report. It returns `Ok(true)` when
/// every invariant passed on every data set, every metric, and every
/// aggregation model. It returns `Ok(false)` when at least one
/// invariant failed. It returns `Err` only for a real failure to run
/// the report, such as a missing WASM file or a broken call into the
/// module.
fn run() -> Result<bool> {
    let wasm_path = scoring::resolve_wasm_path();
    if !wasm_path.exists() {
        anyhow::bail!(
            "the wasm file is not at {}. Run this command first: \
             cargo build --release --target wasm32-unknown-unknown -p eval-script",
            wasm_path.display()
        );
    }

    let mut instance = ScriptInstance::load(&wasm_path)
        .with_context(|| format!("cannot load the wasm module at {}", wasm_path.display()))?;

    // The response seed must differ from the data set seed. Read the
    // trap note on `archetype::responses_for`. `ResponseSeed::derive`
    // applies the fixed mask that keeps the two streams apart.
    let response_seed = ResponseSeed::derive(SEED);

    print_header(&wasm_path, &instance, response_seed);

    let mut failed_invariants: Vec<FailedInvariant> = Vec::new();

    for shape in DatasetShape::ALL {
        run_shape(&mut instance, shape, response_seed, &mut failed_invariants)?;
    }

    run_gap_ladder(&mut instance)?;

    print_summary(&failed_invariants);

    Ok(failed_invariants.is_empty())
}

/// This function prints the header block of the report.
fn print_header(wasm_path: &Path, instance: &ScriptInstance, response_seed: ResponseSeed) {
    println!("MINER SIMULATOR REPORT");
    println!("======================");
    println!("wasm path: {}", wasm_path.display());
    println!("wasm file size: {} bytes", instance.file_size_bytes());
    println!("instantiation path: {}", instance.instantiation_path());
    println!("seed: {SEED:#x}");
    println!("response seed: {:#x}", response_seed.get());
    println!("items per data set: {ITEMS}");
    println!("bootstrap resamples: {RESAMPLES}");
    println!("score direction: a HIGH score is good. 1.0 is the best score, 0.0 is the worst.");
    println!();
}

/// This function runs the full report for one data set shape.
///
/// It builds one data set, makes responses for every archetype, scores
/// every archetype with the Brier metric, and builds standings under
/// both aggregation models. It prints the standings side by side, the
/// ejected miner list, the order comparison, and the verdict blocks for
/// both models, then the bootstrap rank flip table. On the skewed shape
/// it also prints the Brier Skill Score table. It records every failed
/// invariant into the caller's list.
fn run_shape(
    instance: &mut ScriptInstance,
    shape: DatasetShape,
    response_seed: ResponseSeed,
    failed_invariants: &mut Vec<FailedInvariant>,
) -> Result<()> {
    let shape_name = shape.name();
    let ds = dataset::generate(shape, ITEMS, SEED);

    println!("DATA SET: {shape_name}");
    println!("----------{}", "-".repeat(shape_name.len()));
    println!("n: {}", ds.items.len());
    println!("realised base rate: {:.6}", ds.realised_base_rate);
    println!("majority label: {}", ds.majority_label);
    println!("hard signal threshold: {:.6}", ds.hard_signal_threshold);
    println!();

    let mut brier_results = Vec::with_capacity(Archetype::ALL.len());
    for arch in Archetype::ALL {
        let responses = archetype::responses_for(arch, &ds, response_seed);
        let result = scoring::score_miner(instance, &ds, arch, &responses, Metric::Brier)
            .with_context(|| {
                format!(
                    "cannot score archetype {} on shape {shape_name}",
                    arch.name(),
                )
            })?;
        brier_results.push(result);
    }

    let brier_sak = leaderboard::build_standings(&brier_results, AggregationModel::ScoreAndKeep);
    let brier_eject = leaderboard::build_standings(&brier_results, AggregationModel::Eject);

    println!(
        "{}",
        leaderboard::render_standings_side_by_side(
            &brier_sak,
            &brier_eject,
            "BRIER STANDINGS: score_and_keep vs eject (higher is better)"
        )
    );
    println!(
        "{}",
        leaderboard::compare_orderings(&brier_sak, &brier_eject)
    );
    println!(
        "{}",
        leaderboard::render_ejected(&brier_eject, "BRIER EJECTED (eject model)")
    );

    let brier_sak_lines = verdict::check_all(&brier_sak.ranked);
    let brier_eject_lines = verdict::check_all_eject(&brier_eject);

    println!(
        "{}",
        verdict::render(&brier_sak_lines, "VERDICT (brier, score_and_keep)")
    );
    println!(
        "note: the eject verdict below restates invariants 6 and 7. they now test the \
         AGGREGATION layer (does the protocol remove the failing miner), not the scoring rule."
    );
    println!(
        "{}",
        verdict::render(&brier_eject_lines, "VERDICT (brier, eject)")
    );

    for (model_name, lines) in [
        ("score_and_keep", &brier_sak_lines),
        ("eject", &brier_eject_lines),
    ] {
        for line in lines {
            if !line.passed {
                failed_invariants.push(FailedInvariant {
                    shape_name,
                    metric_name: Metric::Brier.name(),
                    model_name,
                    line: line.clone(),
                });
            }
        }
    }

    let brier_flips = bootstrap::rank_flips(&brier_results, &brier_sak.ranked, RESAMPLES, SEED);

    println!(
        "{}",
        bootstrap::render_flips(
            &brier_flips,
            "BOOTSTRAP RANK FLIPS (brier, score_and_keep model)"
        )
    );

    if shape == DatasetShape::Skewed {
        let skill_rows = scoring::build_skill_table(&brier_results, &ds);
        println!(
            "{}",
            scoring::render_skill_table(&skill_rows, "BRIER SKILL SCORE TABLE (skewed data set)")
        );
    }

    Ok(())
}

/// This function runs the top two gap ladder on the balanced shape and
/// prints the result. It loops over `Metric::ALL`, which today holds
/// only `Metric::Brier`, so the report stays ready for a second metric
/// without a rewrite if the protocol ever publishes one.
///
/// The gap and the confidence range both use `mean(top) - mean(second)`,
/// so a positive value always means the top archetype truly scores
/// higher. Read the note on score direction at the top of this file.
fn run_gap_ladder(instance: &mut ScriptInstance) -> Result<()> {
    println!("TOP TWO GAP VS SAMPLE SIZE (balanced shape, higher score is better)");
    println!("---------------------------------------------------------------------");

    for metric in Metric::ALL {
        let reports = bootstrap::top_two_gap_at_sizes(
            instance,
            DatasetShape::Balanced,
            &Archetype::ALL,
            metric,
            &GAP_LADDER,
            RESAMPLES,
            SEED,
        )
        .with_context(|| format!("cannot run the gap ladder for metric {}", metric.name()))?;

        println!(
            "{}",
            bootstrap::render_gap_reports(
                &reports,
                &format!("gap ladder, metric {}", metric.name())
            )
        );

        match reports.iter().find(|r| r.separated) {
            Some(first_separated) => println!(
                "smallest sample size with a separated gap, metric {}: {}",
                metric.name(),
                first_separated.sample_size
            ),
            None => println!(
                "no sample size in the ladder had a separated gap, metric {}",
                metric.name()
            ),
        }
        println!();
    }

    Ok(())
}

/// This function prints the final summary block of the report.
///
/// It lists every failed invariant, with the data set shape, the
/// metric, the aggregation model, and the numbers. It prints a clear
/// pass or fail line at the end.
fn print_summary(failed_invariants: &[FailedInvariant]) {
    println!("SUMMARY");
    println!("=======");

    if failed_invariants.is_empty() {
        println!(
            "every invariant passed on every data set, every metric, and every aggregation model."
        );
    } else {
        println!("{} invariant check(s) failed:", failed_invariants.len());
        for failure in failed_invariants {
            println!(
                "  shape={} metric={} model={} #{}: {} | {}",
                failure.shape_name,
                failure.metric_name,
                failure.model_name,
                failure.line.number,
                failure.line.statement,
                failure.line.detail
            );
        }
    }
    println!();

    if failed_invariants.is_empty() {
        println!("RESULT: PASS");
    } else {
        println!("RESULT: FAIL");
    }
}
