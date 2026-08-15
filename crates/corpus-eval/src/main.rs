//! `corpus-eval`: run the scoring module over the real corpus and
//! produce the evaluation tables.
//!
//! This is a HOST-SIDE tool: plain std, native, no wasm inside it. The
//! wasm boundary runs in `tools/wazero-runner`; this tool prepares the
//! input for that runner and reduces its output into tables.
//!
//! ```text
//! cargo run -p corpus-eval -- crossbranch
//! ```

mod adversarial;
mod baseline;
mod bootstrap;
mod corpus;
mod coverage;
mod crossbranch;
mod geocode;
mod h2hreport;
mod headtohead;
mod knownbad;
mod ranking;
mod stats;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("help");
    match command {
        "adversarial-emit" => {
            let output = std::path::Path::new("corpus/adversarial-input.jsonl");
            match adversarial::emit(output) {
                Ok(count) => {
                    println!("wrote {count} adversarial cases to {}", output.display());
                    println!("now score them with the wazero runner, then run adversarial-report");
                }
                Err(error) => {
                    eprintln!("cannot write the adversarial cases: {error}");
                    std::process::exit(1);
                }
            }
        }
        "adversarial-report" => {
            let path = std::path::Path::new("corpus/adversarial-scores.jsonl");
            match stats::load_scores(path) {
                Ok(rows) => {
                    if let Err(error) = adversarial::print_report(&rows) {
                        eprintln!("the adversarial report failed: {error}");
                        std::process::exit(1);
                    }
                }
                Err(error) => {
                    eprintln!("cannot read the adversarial scores: {error}");
                    std::process::exit(1);
                }
            }
        }
        "geocode" => {
            let plan_path = std::path::Path::new("corpus/batch-plan.json");
            let coords_path = std::path::Path::new(geocode::COORDS_PATH);
            println!("=== GEOCODING THE BATCH CITY LIST ===");
            println!("coordinates come from this list, never from a miner response");
            println!();
            match geocode::load_plan(plan_path)
                .and_then(|plan| geocode::resolve_plan(&plan, coords_path))
            {
                Ok(coords) => {
                    println!();
                    println!(
                        "{} cities resolved -> {}",
                        coords.len(),
                        geocode::COORDS_PATH
                    );
                }
                Err(error) => {
                    eprintln!("geocoding failed: {error}");
                    std::process::exit(1);
                }
            }
        }
        "headtohead" => {
            if let Err(error) = run_head_to_head() {
                eprintln!("head-to-head failed: {error}");
                std::process::exit(1);
            }
        }
        "crossbranch" => crossbranch::print_table(),
        "renderings" => crossbranch::print_renderings(),
        "separation" => crossbranch::print_separation(),
        "parsecov" => {
            let prepared = std::path::Path::new("corpus/eval-input.jsonl");
            let raw = std::path::Path::new("corpus/weather-triples.jsonl");
            match corpus::load_eval_rows(prepared) {
                Ok(rows) => {
                    println!("=== PARSER COVERAGE ON REAL CORPUS TEXT ===");
                    println!();
                    let extracted = coverage::measure(
                        "extracted miner value (what rank_answer really receives)",
                        rows.iter().map(|row| row.miner_value.as_str()),
                    );
                    coverage::print_report(&extracted, 20);

                    let bare = coverage::measure(
                        "ground truth, bare rendering",
                        rows.iter().map(|row| row.gt_bare.as_str()),
                    );
                    coverage::print_report(&bare, 20);

                    let prose = coverage::measure(
                        "ground truth, prose rendering",
                        rows.iter().map(|row| row.gt_prose.as_str()),
                    );
                    coverage::print_report(&prose, 20);

                    let json_gt = coverage::measure(
                        "ground truth, JSON rendering",
                        rows.iter().map(|row| row.gt_json.as_str()),
                    );
                    coverage::print_report(&json_gt, 20);

                    let questions = coverage::measure(
                        "question text (advisory input, never required)",
                        rows.iter().map(|row| row.question.as_str()),
                    );
                    coverage::print_report(&questions, 20);

                    match corpus::raw_miner_answers(raw) {
                        Ok(answers) => {
                            let raw_report = coverage::measure(
                                "RAW upstream miner response (rank_answer never sees this)",
                                answers.iter().map(String::as_str),
                            );
                            coverage::print_report(&raw_report, 20);
                        }
                        Err(error) => eprintln!("cannot read raw answers: {error}"),
                    }
                }
                Err(error) => {
                    eprintln!("cannot read prepared rows: {error}");
                    std::process::exit(1);
                }
            }
        }
        "knownbad" => {
            let scores = std::path::Path::new("corpus/eval-scores.jsonl");
            let corpus = std::path::Path::new("corpus/weather-triples.jsonl");
            match stats::load_scores(scores) {
                Ok(rows) => {
                    if let Err(error) = knownbad::print_table(corpus, &rows) {
                        eprintln!("known-bad report failed: {error}");
                        std::process::exit(1);
                    }
                }
                Err(error) => {
                    eprintln!("cannot read scores: {error}");
                    std::process::exit(1);
                }
            }
        }
        "rankflip" => {
            // An optional path, so the same reduction runs over the
            // head-to-head scores without a second code path.
            let path_arg = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| "corpus/eval-scores.jsonl".to_string());
            let path = std::path::Path::new(&path_arg);
            match stats::load_scores(path) {
                Ok(rows) => ranking::print_rank_flips(&rows, 2000, 20260814),
                Err(error) => {
                    eprintln!("cannot read scores: {error}");
                    std::process::exit(1);
                }
            }
        }
        "stats" => {
            // An optional path, so the same reduction runs over the
            // head-to-head scores without a second code path.
            let path_arg = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| "corpus/eval-scores.jsonl".to_string());
            let path = std::path::Path::new(&path_arg);
            match stats::load_scores(path) {
                Ok(rows) => {
                    stats::print_rendering_variance(&rows);
                    println!();
                    stats::print_miner_stats(&rows);
                }
                Err(error) => {
                    eprintln!("cannot read scores: {error}");
                    std::process::exit(1);
                }
            }
        }
        "prepare" => {
            // Optional paths, so the same preparation runs over the
            // head-to-head corpus without a second code path.
            let input_arg = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| "corpus/weather-triples.jsonl".to_string());
            let output_arg = args
                .get(3)
                .cloned()
                .unwrap_or_else(|| "corpus/eval-input.jsonl".to_string());
            let input = std::path::Path::new(&input_arg);
            let output = std::path::Path::new(&output_arg);
            match corpus::prepare(input, output) {
                Ok(report) => {
                    println!("rows read:           {}", report.rows_read);
                    println!("rows with truth:     {}", report.rows_with_truth);
                    println!("rows written:        {}", report.rows_written);
                    println!("drops by reason:");
                    for (reason, count) in &report.drop_reasons {
                        println!("  {reason:28} {count}");
                    }
                    println!("wrote {}", output.display());
                }
                Err(error) => {
                    eprintln!("prepare failed: {error}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!(
                "usage: corpus-eval <crossbranch|renderings|separation|prepare|stats|parsecov|\
                 rankflip|knownbad|adversarial-emit|adversarial-report|geocode|headtohead>"
            );
            std::process::exit(2);
        }
    }
}

/// This function joins ground truth onto the bought asks and reports.
///
/// The truth comes from the geocoded city list and the client-side ask
/// time. No coordinate and no timestamp used for the join comes out of
/// a miner response.
fn run_head_to_head() -> Result<(), String> {
    use std::collections::{BTreeMap, BTreeSet};

    let asks = headtohead::load_asks(std::path::Path::new("corpus/ask-batch.jsonl"))?;
    let coordinates = geocode::load_coordinates(std::path::Path::new(geocode::COORDS_PATH))?;

    println!("=== HEAD-TO-HEAD GROUND TRUTH JOIN ===");
    println!("asks read:   {}", asks.len());
    println!("cities:      {}", coordinates.len());

    let (start, end) = headtohead::date_span(&asks);
    println!("archive span {start} to {end}");
    println!();

    let mut archives = BTreeMap::new();
    for key in headtohead::city_keys(&asks) {
        let Some(place) = coordinates.get(&key) else {
            println!("  {key:<12} NOT GEOCODED, skipped");
            continue;
        };
        match headtohead::fetch_archive(place.latitude, place.longitude, &start, &end) {
            Ok(series) => {
                println!("  {key:<12} archive fetched");
                archives.insert(key, series);
            }
            Err(error) => println!("  {key:<12} archive FAILED: {error}"),
        }
    }

    let mut drops = headtohead::DropCounts::default();
    let rows = headtohead::join(&asks, &coordinates, &archives, &mut drops);

    let output = std::path::Path::new(headtohead::OUTPUT_PATH);
    headtohead::write_rows(&rows, output)?;
    println!();
    println!("rows written: {} -> {}", rows.len(), output.display());

    let settled_units: u64 = asks.iter().filter(|ask| ask.settled).count() as u64 * 10_000;
    let observed: BTreeSet<String> = asks.iter().filter_map(|ask| ask.miner_id.clone()).collect();

    h2hreport::print_report(&rows, 20, 7.0 / 30.0, &drops, settled_units, &observed);
    Ok(())
}
