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

mod baseline;
mod bootstrap;
mod corpus;
mod coverage;
mod crossbranch;
mod knownbad;
mod ranking;
mod stats;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("help");
    match command {
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
            let path = std::path::Path::new("corpus/eval-scores.jsonl");
            match stats::load_scores(path) {
                Ok(rows) => ranking::print_rank_flips(&rows, 2000, 20260814),
                Err(error) => {
                    eprintln!("cannot read scores: {error}");
                    std::process::exit(1);
                }
            }
        }
        "stats" => {
            let path = std::path::Path::new("corpus/eval-scores.jsonl");
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
            let input = std::path::Path::new("corpus/weather-triples.jsonl");
            let output = std::path::Path::new("corpus/eval-input.jsonl");
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
                "usage: corpus-eval <crossbranch|renderings|separation|prepare|stats|parsecov|rankflip|knownbad>"
            );
            std::process::exit(2);
        }
    }
}
