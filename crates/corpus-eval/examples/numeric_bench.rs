//! Can a scorer tell a near miss from a catastrophe?
//!
//! When the wanted answer is a QUANTITY, the difference between a good
//! miner and a bad one is a difference of degree, not of kind. A price
//! of 192.44 against a truth of 192.43 is one cent out. A price of
//! 999999.99 is not a price. Both are "wrong" to a scorer that compares
//! text, and a sentence embedder has no notion that the first two
//! numbers are 0.005 percent apart.
//!
//! This benchmark measures that. It is the evidence behind the claim
//! that a numeric scorer earns its place next to a 22 MB transformer.
//!
//! # Two kinds of row
//!
//! - A LADDER holds one ground truth and answers at growing error. The
//!   SHAPE of the curve is the evidence, not any single value. Two
//!   numbers come out of it: the span from the best answer to the worst,
//!   and the count of INVERSIONS, which are pairs where a larger error
//!   scores higher than a smaller one. A scorer with inversions pays a
//!   miner to be more wrong.
//! - A PAIR holds one truth, one answer that is objectively closer and
//!   one that is objectively further. A module either separates the pair
//!   or it does not. A tie counts as a failure, the same way the
//!   promotion harness counts one.
//!
//! # What was measured against the deployed champion
//!
//! `telegraphprotocol/telegraph-wasm-baseline` builds two ways. The
//! DEPLOYED one is the default build: 17,952 bytes raw, 8,941 gzipped,
//! which is the ~9.2 KB the protocol reports. That build is projection
//! mode, whose own documentation says its embeddings are "not
//! semantically meaningful" because token IDs are hashed into
//! pseudo-embeddings rather than run through the model.
//!
//! Every figure here is against that deployed build. An earlier
//! revision of this comment quoted the `real_weights` build, which runs
//! actual MiniLM-L6-v2 inference and is a stronger opponent; those
//! numbers described a module nobody is running.
//!
//! | | this module, weather | deployed | `real_weights` |
//! |---|---|---|---|
//! | price ladder span | 1.000000 | 0.244388 | 0.311240 |
//! | price ladder inversions | 0 of 55 | 10 of 55 | 8 of 55 |
//! | temperature ladder span | 0.999699 | 0.087342 | 0.108984 |
//! | temperature ladder inversions | 0 of 55 | 29 of 55 | 11 of 55 |
//! | pairs separated | 19 of 21 | 13 of 21 | 18 of 21 |
//! | mean pair margin | 0.6902 | 0.0328 | 0.0939 |
//!
//! Against the deployed build there is no pair the champion separates
//! and this module does not. The two it fails, 15 and 16, are the unit
//! conversions outside the closed unit set, and it fails those too.
//!
//! The composite gives 0.15 to a BM25 term whose tokeniser splits on
//! non-alphanumeric characters and drops every token shorter than two
//! characters. So `41.3` and `41.8` both reduce to the single term `41`
//! and score identically, and a CVSS of `9.8` reduces to nothing at all
//! and scores 0.0000 against itself. That holds in both builds, since
//! the lexical term does not depend on the weights. Read through the
//! module's own `bm25_score` export.
//!
//! # Running it
//!
//! ```text
//! cargo run -p corpus-eval --release --example numeric_bench -- --report \
//!    --module target/wasm32-unknown-unknown/release/eval_script.wasm \
//!    --champion reference/scoring_module.wasm
//! ```
//!
//! Both modules are scored through the same engine over the same
//! vectors, so the comparison holds for any champion. Nothing here
//! knows what either module does inside.

/// Where `--emit` writes the vectors and `--report` reads them.
const VECTORS_PATH: &str = "target/numeric-vectors.json";
/// Where the engine writes the scores of the module under test.
const SCORES_PATH: &str = "target/numeric-module.json";
/// Where the engine writes the champion's scores.
const CHAMPION_SCORES_PATH: &str = "target/numeric-champion.json";
/// The module scored when the caller names none.
const DEFAULT_MODULE: &str = "target/wasm32-unknown-unknown/release/eval_script.wasm";
/// The champion used when the caller names none.
const DEFAULT_CHAMPION: &str = "reference/scoring_module.wasm";
/// The directory of the engine runner.
const ENGINE_DIR: &str = "tools/wazero-runner";

/// The question a price row carries.
const Q_PRICE: &str = "[direct] 207 -> /price";
/// The question a weather row carries.
const Q_TEMP: &str = "[direct] 211 -> /weather";
/// The question a gas row carries.
const Q_GAS: &str = "[direct] 219 -> /gas";

/// One rung of a ladder.
struct Rung {
    /// The short label for the table.
    label: &'static str,
    /// The answer text at this error.
    answer: &'static str,
}

/// One ladder: a truth, and answers at growing error.
struct Ladder {
    /// The short name for the table.
    name: &'static str,
    /// The question text.
    question: &'static str,
    /// The ground truth text.
    truth: &'static str,
    /// The rungs, ordered from the smallest error to the largest.
    rungs: &'static [Rung],
}

/// The price ladder, truth `$192.43`.
///
/// The answers are written out rather than computed, so that the file
/// carries the exact bytes the engine scores and a reader does not have
/// to run the code to know what was asked.
const PRICE_RUNGS: [Rung; 11] = [
    Rung {
        label: "exact",
        answer: "192.43 USD",
    },
    Rung {
        label: "5e-5",
        answer: "192.44 USD",
    },
    Rung {
        label: "1e-4",
        answer: "192.45 USD",
    },
    Rung {
        label: "1e-3",
        answer: "192.62 USD",
    },
    Rung {
        label: "1e-2",
        answer: "194.35 USD",
    },
    Rung {
        label: "5e-2",
        answer: "202.05 USD",
    },
    Rung {
        label: "1e-1",
        answer: "211.67 USD",
    },
    Rung {
        label: "5e-1",
        answer: "288.64 USD",
    },
    Rung {
        label: "2x",
        answer: "384.86 USD",
    },
    Rung {
        label: "11x",
        answer: "2116.73 USD",
    },
    Rung {
        label: "5200x",
        answer: "999866.28 USD",
    },
];

/// The temperature ladder, truth `The temperature is 28.9 C.`
const TEMP_RUNGS: [Rung; 11] = [
    Rung {
        label: "exact",
        answer: "28.90 C",
    },
    Rung {
        label: "+0.01",
        answer: "28.91 C",
    },
    Rung {
        label: "+0.05",
        answer: "28.95 C",
    },
    Rung {
        label: "+0.1",
        answer: "29.00 C",
    },
    Rung {
        label: "+0.3",
        answer: "29.20 C",
    },
    Rung {
        label: "+0.5",
        answer: "29.40 C",
    },
    Rung {
        label: "+1.0",
        answer: "29.90 C",
    },
    Rung {
        label: "+2.0",
        answer: "30.90 C",
    },
    Rung {
        label: "+5.0",
        answer: "33.90 C",
    },
    Rung {
        label: "+10.0",
        answer: "38.90 C",
    },
    Rung {
        label: "+50.0",
        answer: "78.90 C",
    },
];

/// The ladders.
const LADDERS: [Ladder; 2] = [
    Ladder {
        name: "price",
        question: Q_PRICE,
        truth: "$192.43",
        rungs: &PRICE_RUNGS,
    },
    Ladder {
        name: "temperature",
        question: Q_TEMP,
        truth: "The temperature is 28.9 C.",
        rungs: &TEMP_RUNGS,
    },
];

/// One discrimination pair.
struct Pair {
    /// The family of case the pair belongs to.
    family: &'static str,
    /// The question text.
    question: &'static str,
    /// The ground truth text.
    truth: &'static str,
    /// The label of the answer that is objectively closer.
    close_label: &'static str,
    /// The answer that is objectively closer.
    close: &'static str,
    /// The label of the answer that is objectively further.
    far_label: &'static str,
    /// The answer that is objectively further.
    far: &'static str,
    /// What the pair proves.
    note: &'static str,
}

/// The pairs.
///
/// Every pair has an answer that is closer to the truth by QUANTITY and
/// one that is further. Two of them, the unit conversions this module
/// has no entry for, are expected losses and are kept for that reason.
const PAIRS: [Pair; 21] = [
    Pair {
        family: "near-vs-far",
        question: Q_PRICE,
        truth: "$192.43",
        close_label: "near 192.44",
        close: "192.44 USD",
        far_label: "far 999999.99",
        far: "999999.99 USD",
        note: "0.005 percent out against five thousand times out",
    },
    Pair {
        family: "near-vs-far",
        question: Q_PRICE,
        truth: "$192.43",
        close_label: "near 192.44",
        close: "192.44 USD",
        far_label: "far 210.00",
        far: "210.00 USD",
        note: "0.005 percent out against 9 percent out",
    },
    Pair {
        family: "near-vs-far",
        question: Q_TEMP,
        truth: "The temperature is 28.9 C.",
        close_label: "near 28.91",
        close: "28.91 C",
        far_label: "far 31.5",
        far: "31.5 C",
        note: "0.03 percent out against 9 percent out",
    },
    Pair {
        family: "near-vs-far",
        question: Q_TEMP,
        truth: "The temperature is 41.3 C.",
        close_label: "near 41.3",
        close: "41.3 C",
        far_label: "far 41.8",
        far: "41.8 C",
        note: "one decimal apart, and BM25 reduces both to the term 41",
    },
    Pair {
        family: "near-vs-far",
        question: Q_GAS,
        truth: "12 gwei",
        close_label: "near 12",
        close: "12 gwei",
        far_label: "far 40",
        far: "40 gwei",
        note: "exact against three times out",
    },
    Pair {
        family: "magnitude",
        question: Q_PRICE,
        truth: "$192.43",
        close_label: "exact",
        close: "192.43 USD",
        far_label: "10x 1924.3",
        far: "1924.30 USD",
        note: "one decimal point moved is ten times the money",
    },
    Pair {
        family: "magnitude",
        question: Q_TEMP,
        truth: "The temperature is 1.5 C.",
        close_label: "exact 1.5",
        close: "1.5 C",
        far_label: "10x 15",
        far: "15 C",
        note: "one character difference, ten times the quantity",
    },
    Pair {
        family: "magnitude",
        question: Q_TEMP,
        truth: "The temperature is 0.15 C.",
        close_label: "exact 0.15",
        close: "0.15 C",
        far_label: "100x 15",
        far: "15 C",
        note: "two characters, a hundred times the quantity",
    },
    Pair {
        family: "sign",
        question: Q_TEMP,
        truth: "The temperature is -5.0 C.",
        close_label: "exact -5.0",
        close: "-5.0 C",
        far_label: "sign flipped 5.0",
        far: "5.0 C",
        note: "one character, ten degrees, freezing against mild",
    },
    Pair {
        family: "format-same",
        question: Q_PRICE,
        truth: "$192.43",
        close_label: "bare 192.43",
        close: "192.43",
        far_label: "wrong 192.53",
        far: "192.53",
        note: "a correct bare value against a value ten cents out",
    },
    Pair {
        family: "format-same",
        question: Q_PRICE,
        truth: "$192.43",
        close_label: "prose exact",
        close: "The price is $192.43 per token.",
        far_label: "prose wrong",
        far: "The price is $210.00 per token.",
        note: "the same sentence around a right and a wrong number",
    },
    Pair {
        family: "format-same",
        question: Q_PRICE,
        truth: "$192.43",
        close_label: "sci 1.9243e2",
        close: "1.9243e2 USD",
        far_label: "sci 2.1e2",
        far: "2.1e2 USD",
        note: "scientific notation, right against wrong",
    },
    Pair {
        family: "format-same",
        question: Q_PRICE,
        truth: "$192.43",
        close_label: "trailing zero",
        close: "192.430 USD",
        far_label: "transposed",
        far: "192.34 USD",
        note: "a cosmetic zero against two transposed digits",
    },
    Pair {
        family: "format-same",
        question: Q_PRICE,
        truth: "$1234.56",
        close_label: "grouped exact",
        close: "1,234.56 USD",
        far_label: "grouped wrong",
        far: "1,254.56 USD",
        note: "a thousands separator on a right and a wrong value",
    },
    Pair {
        family: "unit-convert",
        question: Q_TEMP,
        truth: "The temperature is 28.9 C.",
        close_label: "correct in F",
        close: "84.02 F",
        far_label: "wrong in F",
        far: "95.0 F",
        note: "a right conversion against a wrong one, and F is in the unit set",
    },
    Pair {
        family: "unit-convert",
        question: Q_GAS,
        truth: "12 gwei",
        close_label: "correct in ETH",
        close: "0.000000012 ETH",
        far_label: "wrong in ETH",
        far: "0.000000040 ETH",
        note: "EXPECTED LOSS: ETH is outside the closed unit set, so both tie",
    },
    Pair {
        family: "unit-convert",
        question: "[direct] 231 -> /distance",
        truth: "1000 m",
        close_label: "correct in km",
        close: "1 km",
        far_label: "wrong in km",
        far: "4 km",
        note: "EXPECTED LOSS: metres are outside the closed unit set",
    },
    Pair {
        family: "spray",
        question: Q_PRICE,
        truth: "$192.43",
        close_label: "single exact",
        close: "192.43 USD",
        far_label: "six values incl. exact",
        far: "192.40 192.41 192.42 192.43 192.44 192.45 USD",
        note: "one right value against a spread that contains it",
    },
    Pair {
        family: "spray",
        question: Q_TEMP,
        truth: "The temperature is 28.9 C.",
        close_label: "single exact",
        close: "28.9 C",
        far_label: "range containing it",
        far: "somewhere between 20 C and 35 C, likely 25 C or 30 C",
        note: "one right value against a hedge that brackets it",
    },
    Pair {
        family: "padding",
        question: Q_PRICE,
        truth: "$192.43",
        close_label: "short exact",
        close: "192.43 USD",
        far_label: "padded wrong",
        far: "After reviewing the market data across several exchanges the \
              current quoted price for this asset is approximately 210.00 USD.",
        note: "a right short answer against a wrong long one",
    },
    Pair {
        family: "padding",
        question: Q_TEMP,
        truth: "The temperature is 28.9 C.",
        close_label: "short exact",
        close: "28.9 C",
        far_label: "padded wrong",
        far: "Based on the latest observation from the nearest weather station \
              the air temperature at this location is about 31.5 C right now.",
        note: "a right short answer against a wrong long one",
    },
];

// -----------------------------------------------------------------
// Vectors
// -----------------------------------------------------------------

/// This function writes every vector in the golden-vector shape.
///
/// The shape is what `tools/wazero-runner -golden` reads, so any module
/// that exports the published ABI can be scored over exactly these
/// bytes.
fn write_vectors() -> std::io::Result<usize> {
    let mut vectors = Vec::new();
    for ladder in LADDERS.iter() {
        for rung in ladder.rungs {
            vectors.push(serde_json::json!({
                "name": format!("{}::{}", ladder.name, rung.label),
                "question": ladder.question,
                "ground_truth": ladder.truth,
                "miner_answer": rung.answer,
                "expected": 0.0,
            }));
        }
    }
    for (index, pair) in PAIRS.iter().enumerate() {
        for (suffix, answer) in [("close", pair.close), ("far", pair.far)] {
            vectors.push(serde_json::json!({
                "name": format!("pair{index:02}::{suffix}"),
                "question": pair.question,
                "ground_truth": pair.truth,
                "miner_answer": answer,
                "expected": 0.0,
            }));
        }
    }
    let count = vectors.len();
    let document = serde_json::json!({ "vectors": vectors });
    std::fs::write(
        VECTORS_PATH,
        format!("{}\n", serde_json::to_string_pretty(&document)?),
    )?;
    Ok(count)
}

/// This function runs one module over the vectors and reads its scores.
///
/// It gives `None`, with a message, when the run fails.
fn run_module_scores(wasm: &str, out: &str) -> Option<std::collections::HashMap<String, f64>> {
    // The engine runner runs from its own directory, so a relative path
    // here needs the step back up to the workspace root.
    let from_runner = |path: &str| -> String {
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("../../{path}")
        }
    };

    let outcome = std::process::Command::new("go")
        .args([
            "run",
            ".",
            "-golden",
            &from_runner(VECTORS_PATH),
            "-a",
            &from_runner(wasm),
            "-out",
            &from_runner(out),
        ])
        .current_dir(ENGINE_DIR)
        .output();

    let outcome = match outcome {
        Ok(value) => value,
        Err(error) => {
            println!("cannot start the engine runner: {error}");
            return None;
        }
    };
    if !outcome.status.success() {
        println!(
            "the engine could not score {wasm}: {}",
            String::from_utf8_lossy(&outcome.stderr).trim()
        );
        return None;
    }

    let text = std::fs::read_to_string(out).ok()?;
    let document: serde_json::Value = serde_json::from_str(&text).ok()?;
    let mut table = std::collections::HashMap::new();
    for row in document["vectors"].as_array()? {
        let name = row["name"].as_str()?.to_string();
        table.insert(name, row["value"].as_f64()?);
    }
    Some(table)
}

// -----------------------------------------------------------------
// The report
// -----------------------------------------------------------------

/// This function counts the pairs of rungs where a LARGER error scores
/// higher than a smaller one.
///
/// The rungs arrive ordered from the smallest error to the largest, so
/// a correct scorer gives a list that never rises. Every rise is a case
/// where the module pays a miner to be more wrong. The comparison
/// carries a small tolerance, because two rungs that score the same are
/// a failure to separate, not an inversion.
fn inversions(values: &[f64]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (i, earlier) in values.iter().enumerate() {
        for (j, later) in values.iter().enumerate().skip(i + 1) {
            if *later > *earlier + 1e-9 {
                out.push((i, j));
            }
        }
    }
    out
}

/// This function prints one ladder for both modules.
fn print_ladder(
    ladder: &Ladder,
    module: &std::collections::HashMap<String, f64>,
    champion: &std::collections::HashMap<String, f64>,
) {
    println!("   {} ladder, truth {:?}", ladder.name, ladder.truth);
    println!(
        "   {:<8} {:<24} {:>12} {:>12}",
        "error", "answer", "ours", "champion"
    );
    let mut ours = Vec::new();
    let mut theirs = Vec::new();
    for rung in ladder.rungs {
        let key = format!("{}::{}", ladder.name, rung.label);
        let a = *module.get(&key).unwrap_or(&-1.0);
        let b = *champion.get(&key).unwrap_or(&-1.0);
        ours.push(a);
        theirs.push(b);
        println!(
            "   {:<8} {:<24} {:>12.6} {:>12.6}",
            rung.label, rung.answer, a, b
        );
    }

    let span = |values: &[f64]| -> f64 {
        let high = values.iter().cloned().fold(f64::MIN, f64::max);
        let low = values.iter().cloned().fold(f64::MAX, f64::min);
        high - low
    };
    let total = ladder.rungs.len() * (ladder.rungs.len() - 1) / 2;
    let ours_inversions = inversions(&ours);
    let their_inversions = inversions(&theirs);
    println!(
        "   {:<33} {:>12.6} {:>12.6}",
        "span, best to worst",
        span(&ours),
        span(&theirs)
    );
    println!(
        "   {:<33} {:>9} /{total} {:>9} /{total}",
        "INVERSIONS, a bigger error paid more",
        ours_inversions.len(),
        their_inversions.len()
    );
    // An inversion is the finding, so it is named and not just counted.
    for (name, list) in [("ours", &ours_inversions), ("champion", &their_inversions)] {
        for (i, j) in list.iter().take(3) {
            println!(
                "      {name}: {} scores below {}",
                ladder.rungs[*i].label, ladder.rungs[*j].label
            );
        }
        if list.len() > 3 {
            println!("      {name}: and {} more", list.len() - 3);
        }
    }
    println!();
}

/// This function prints every pair and the aggregate.
fn print_pairs(
    module: &std::collections::HashMap<String, f64>,
    champion: &std::collections::HashMap<String, f64>,
) {
    println!("2. pairs: does the module rank the closer answer above the further one?");
    println!("   a tie is a failure to separate, the same as a loss");
    println!(
        "   {:<3} {:<13} {:<22} {:<24} {:>9} {:>9} {:>9} {:>9}  who",
        "id", "family", "closer", "further", "ours+", "ours-", "o.margin", "c.margin"
    );

    let mut ours_margins = Vec::new();
    let mut their_margins = Vec::new();
    let mut ours_only = Vec::new();
    let mut champion_only = Vec::new();
    let mut neither = Vec::new();

    for (index, pair) in PAIRS.iter().enumerate() {
        let close = *module
            .get(&format!("pair{index:02}::close"))
            .unwrap_or(&-1.0);
        let far = *module.get(&format!("pair{index:02}::far")).unwrap_or(&-1.0);
        let base_close = *champion
            .get(&format!("pair{index:02}::close"))
            .unwrap_or(&-1.0);
        let base_far = *champion
            .get(&format!("pair{index:02}::far"))
            .unwrap_or(&-1.0);
        let margin = close - far;
        let base_margin = base_close - base_far;
        ours_margins.push(margin);
        their_margins.push(base_margin);

        let we = margin > 0.0;
        let they = base_margin > 0.0;
        let who = match (we, they) {
            (true, true) => "both",
            (true, false) => "OURS ONLY",
            (false, true) => "CHAMPION ONLY",
            (false, false) => "neither",
        };
        match (we, they) {
            (true, false) => ours_only.push(index),
            (false, true) => champion_only.push(index),
            (false, false) => neither.push(index),
            _ => {}
        }

        println!(
            "   {:<3} {:<13} {:<22} {:<24} {:>9.4} {:>9.4} {:>9.4} {:>9.4}  {}",
            index,
            pair.family,
            pair.close_label,
            fit(pair.far_label, 24),
            close,
            far,
            margin,
            base_margin,
            who
        );
    }

    let mean = |values: &[f64]| values.iter().sum::<f64>() / (values.len() as f64);
    let ours_wins = ours_margins.iter().filter(|m| **m > 0.0).count();
    let their_wins = their_margins.iter().filter(|m| **m > 0.0).count();
    println!();
    println!("   pairs                        {}", PAIRS.len());
    println!(
        "   separated, ours              {ours_wins}/{}",
        PAIRS.len()
    );
    println!(
        "   separated, champion          {their_wins}/{}",
        PAIRS.len()
    );
    println!(
        "   mean margin                  ours {:.4}   champion {:.4}",
        mean(&ours_margins),
        mean(&their_margins)
    );
    println!("   only ours separates          {}", name_list(&ours_only));
    println!(
        "   only the champion separates  {}",
        name_list(&champion_only)
    );
    println!("   neither separates            {}", name_list(&neither));
    println!();
    println!("3. what each pair proves");
    for (index, pair) in PAIRS.iter().enumerate() {
        println!("   {index:<3} {}", pair.note);
    }
}

/// This function renders a list of pair indexes.
fn name_list(indexes: &[usize]) -> String {
    if indexes.is_empty() {
        return "none".to_string();
    }
    indexes
        .iter()
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// This function cuts a text to a column width.
fn fit(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return format!("{text:width$}");
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('~');
    out
}

/// This function scores both modules and prints the report.
fn report(module_path: &str, champion_path: &str) -> std::io::Result<()> {
    let count = write_vectors()?;
    println!("module:   {module_path}");
    println!("champion: {champion_path}");
    println!("vectors:  {count}");
    println!();

    let Some(module) = run_module_scores(module_path, SCORES_PATH) else {
        return Ok(());
    };
    let Some(champion) = run_module_scores(champion_path, CHAMPION_SCORES_PATH) else {
        return Ok(());
    };

    println!("1. ladders: one truth, growing error");
    println!("   an inversion is a pair where a BIGGER error scored HIGHER.");
    println!();
    for ladder in LADDERS.iter() {
        print_ladder(ladder, &module, &champion);
    }
    print_pairs(&module, &champion);
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("");
    let value_of = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };

    let outcome = match mode {
        "--emit" => write_vectors().map(|count| {
            println!("wrote {count} vectors to {VECTORS_PATH}");
        }),
        "--report" => report(
            &value_of("--module").unwrap_or_else(|| DEFAULT_MODULE.to_string()),
            &value_of("--champion").unwrap_or_else(|| DEFAULT_CHAMPION.to_string()),
        ),
        _ => {
            println!(
                "usage: numeric_bench --emit \
                 | --report [--module <wasm>] [--champion <wasm>]"
            );
            Ok(())
        }
    };
    if let Err(error) = outcome {
        println!("failed: {error}");
        std::process::exit(1);
    }
}
