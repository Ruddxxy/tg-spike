//! This example shows what the scorer extracts from a prose answer.
//!
//! The node converts a structured miner response to prose with an LLM
//! before `rank_answer` sees it, so a real miner answer is a SENTENCE
//! and not a value. This probe prints, for each answer text: every
//! quantity the scanner found, which one won the match against the
//! ground truth, and the score.
//!
//! ## The three modes
//!
//! ```text
//! cargo run -p eval-script --example prose_probe -- --emit
//! cargo run -p eval-script --example prose_probe -- --show
//! cargo run -p eval-script --example prose_probe
//! ```
//!
//! `--emit` writes the fixtures as prepared rows for the wazero
//! harness. `--show` reads the scores that harness produced and prints
//! two tables and nothing else, for a screenshot. With no flag the
//! probe prints the full diagnostic view. `--display` is accepted as
//! an older name for `--show`.
//!
//! `--show` prints answer, bare, prose, json, n, flag. It does NOT
//! print the SELECTED column: which candidate out of the answer won
//! the match is an internal detail of the scorer, and this output is
//! meant to be published. The diagnostic view keeps that column.
//!
//! ## Where each number comes from
//!
//! `--display` NEVER recomputes a score. It reads the `f32` the
//! compiled module returned under wazero, which is the value the
//! network sees. The full diagnostic view computes natively, in `f64`,
//! because it needs the intermediate selection that a score alone does
//! not reveal.
//!
//! The SELECTED column, in the diagnostic view only, is native. It is
//! not a score: it names which quantity out of the answer won the
//! match, which no harness output records.

use eval_script::score::score_answer;
use eval_script::value::{scan_values, Family, ParsedValue, Unit, MAX_SCANNED_VALUES};

/// The correct answer for every fixture, in three renderings.
const TRUTH_BARE: &str = "28.1";
const TRUTH_PROSE: &str = "The temperature was 28.1 C.";
const TRUTH_JSON: &str = r#"{"temperature_2m":28.1,"time":"2026-08-16T09:00"}"#;

/// A miner 10 percent out. Any farm at or above this is profitable.
const HONEST_BAR: f64 = 0.0831;

/// Where `--emit` writes and `--display` reads. Relative, so no
/// absolute path can reach the screen.
const INPUT_PATH: &str = "corpus/prose-input.jsonl";
const SCORES_PATH: &str = "corpus/prose-scores.jsonl";

/// One fixture: a key, the answer text, and the Celsius value a
/// correct reading of that answer gives.
struct Fixture {
    key: &'static str,
    answer: &'static str,
    wanted_c: f64,
}

/// The answers a converter plausibly produces for a correct miner.
const PLAUSIBLE: [Fixture; 10] = [
    Fixture {
        key: "p01",
        wanted_c: 28.1,
        answer:
            "The current temperature in Tokyo is 28.1 C with patchy rain nearby and 62% humidity.",
    },
    Fixture {
        key: "p02",
        wanted_c: 28.1,
        answer: "It is 28.1 degrees Celsius in Tokyo right now.",
    },
    Fixture {
        key: "p03",
        wanted_c: 28.1,
        answer: "Tokyo: 28.1C, feels like 30.2C, wind 11.2 kph, humidity 62%.",
    },
    Fixture {
        key: "p04",
        wanted_c: 28.0,
        answer: "The temperature is about 28 degrees.",
    },
    Fixture {
        key: "p05",
        wanted_c: 28.1,
        answer: "Conditions in Tokyo are currently patchy rain with temperatures around 28.1 C.",
    },
    Fixture {
        key: "p06",
        wanted_c: 28.1,
        answer: "Currently 82.6 F in Tokyo.",
    },
    Fixture {
        key: "p07",
        wanted_c: 28.1,
        answer: "The temperature in Tokyo is 28.1 C. Yesterday it was 31.4 C.",
    },
    Fixture {
        key: "p08",
        wanted_c: 28.1,
        answer: "Tokyo is at 301.25 K.",
    },
    Fixture {
        key: "p09",
        wanted_c: 28.1,
        answer: "Humidity is 62% and the temperature is 28.1 C.",
    },
    Fixture {
        key: "p10",
        wanted_c: 28.1,
        answer: "It feels like 30.2 C, actual temperature 28.1 C.",
    },
];

/// Answers that assert 34.9 C and carry 28.1 as a different quantity.
/// A correct selection picks 34.9 and the score should be low.
const ADVERSARIAL: [Fixture; 4] = [
    Fixture {
        key: "a01",
        wanted_c: 34.9,
        answer: "It feels like 28.1 C, actual temperature 34.9 C.",
    },
    Fixture {
        key: "a02",
        wanted_c: 34.9,
        answer: "Tokyo: 34.9C, feels like 28.1C, humidity 62%.",
    },
    Fixture {
        key: "a03",
        wanted_c: 34.9,
        answer: "The temperature is 34.9 C, up from 28.1 C this morning.",
    },
    Fixture {
        key: "a04",
        wanted_c: 34.9,
        answer: "Wind 28.1 kph, temperature 34.9 C.",
    },
];

/// The counter-attack: a miner that echoes the ground truth's own
/// vocabulary around a WRONG number. If the context rule can be farmed,
/// it is farmed here.
const COUNTER: [Fixture; 5] = [
    Fixture {
        key: "c01",
        wanted_c: 34.9,
        answer: "The temperature was 34.9 C.",
    },
    Fixture {
        key: "c02",
        wanted_c: 34.9,
        answer: "temperature 34.9 C, wind 28.1 kph",
    },
    Fixture {
        key: "c03",
        wanted_c: 34.9,
        answer: "The temperature was 34.9 C, wind 28.1 kph.",
    },
    Fixture {
        key: "c04",
        wanted_c: 28.1,
        answer: "The temperature was 28.1 C.",
    },
    Fixture {
        key: "c05",
        wanted_c: 34.9,
        answer: "The temperature was 34.9 C and the temperature was 28.1 C.",
    },
];

fn unit_name(unit: Unit) -> &'static str {
    match unit {
        Unit::None => "",
        Unit::Celsius => "C",
        Unit::Fahrenheit => "F",
        Unit::Kelvin => "K",
        Unit::Percent => "%",
        Unit::Usd => "USD",
        Unit::Eur => "EUR",
        Unit::Gbp => "GBP",
        Unit::Inr => "INR",
        Unit::Jpy => "JPY",
        Unit::Wei => "wei",
        Unit::Gwei => "gwei",
    }
}

fn empty_buffer() -> [ParsedValue; MAX_SCANNED_VALUES] {
    [ParsedValue {
        number: 0.0,
        unit: Unit::None,
    }; MAX_SCANNED_VALUES]
}

/// This function renders one parsed value back to a text the scorer
/// reads as exactly that value.
fn render(value: ParsedValue) -> String {
    match value.unit {
        Unit::None => format!("{}", value.number),
        other => format!("{} {}", value.number, unit_name(other)),
    }
}

/// This function gives the answer quantity that wins the match.
///
/// It repeats the search `score_quantities` runs, so the winner it
/// reports is the one the scorer really used.
fn selected(answer_text: &str) -> Option<ParsedValue> {
    let mut truth = empty_buffer();
    let mut reply = empty_buffer();
    let truth_count = scan_values(TRUTH_BARE, &mut truth);
    let reply_count = scan_values(answer_text, &mut reply);

    let mut best: Option<(ParsedValue, f64)> = None;
    for reply_value in reply.iter().take(reply_count) {
        for truth_value in truth.iter().take(truth_count) {
            let pair = score_answer("", &render(*truth_value), &render(*reply_value));
            if best.map(|(_, score)| pair > score).unwrap_or(true) {
                best = Some((*reply_value, pair));
            }
        }
    }
    best.map(|(value, _)| value)
}

/// This function tells if the selected quantity is the one the answer
/// really asserts.
fn selection_is_right(value: Option<ParsedValue>, wanted_c: f64) -> bool {
    match value {
        Some(value) => {
            let family_ok = matches!(value.family(), Family::Temperature | Family::Dimensionless);
            family_ok && (value.to_base() - wanted_c).abs() < 0.05
        }
        None => false,
    }
}

fn label_for(value: Option<ParsedValue>) -> String {
    match value {
        Some(value) => {
            let unit = unit_name(value.unit);
            if unit.is_empty() {
                format!("{}", value.number)
            } else {
                format!("{} {}", value.number, unit)
            }
        }
        None => "-".to_string(),
    }
}

/// This function cuts a text to a width, counting characters.
fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('~');
    out
}

// ---------------------------------------------------------------
// --emit
// ---------------------------------------------------------------

/// This function escapes a text for a JSON string body.
fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

/// This function writes the fixtures as prepared rows.
///
/// The row shape matches what `tools/wazero-runner` reads in corpus
/// mode, so the fixtures go through the SAME path the published corpus
/// columns use. There is no second harness.
fn emit() -> std::io::Result<()> {
    use std::io::Write;
    let mut text = String::new();
    let mut row_id = 0usize;
    for fixture in PLAUSIBLE
        .iter()
        .chain(ADVERSARIAL.iter())
        .chain(COUNTER.iter())
    {
        text.push_str(&format!(
            "{{\"row_id\":{row_id},\"question\":\"\",\"gt_bare\":\"{}\",\
             \"gt_prose\":\"{}\",\"gt_json\":\"{}\",\"miner_value\":\"{}\",\
             \"miner_slug\":\"prose\",\"intent\":\"WEATHER_CHECK\",\
             \"valid_time\":\"\",\"actual_c\":0.0,\"miner_c\":0.0,\
             \"cluster_id\":\"{}\"}}\n",
            json_escape(TRUTH_BARE),
            json_escape(TRUTH_PROSE),
            json_escape(TRUTH_JSON),
            json_escape(fixture.answer),
            fixture.key,
        ));
        row_id += 1;
    }
    if let Some(parent) = std::path::Path::new(INPUT_PATH).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(INPUT_PATH)?;
    file.write_all(text.as_bytes())?;
    println!("wrote {row_id} rows to {INPUT_PATH}");
    println!("now score them with the wazero runner, then run --display");
    Ok(())
}

// ---------------------------------------------------------------
// --display
// ---------------------------------------------------------------

/// This function pulls one string field out of a JSON object line.
///
/// The harness writes a flat object with no nesting, so a search for
/// the key and then the next quoted run is enough. This crate carries
/// no JSON parser and does not want one for a screenshot helper.
fn field_string(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = line.find(&needle)? + needle.len();
    let rest = line.get(start..)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// This function pulls one number field out of a JSON object line.
fn field_number(line: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{key}\":");
    let start = line.find(&needle)? + needle.len();
    let rest = line.get(start..)?;
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].trim().parse::<f64>().ok()
}

/// The three rendering scores for one fixture, as wazero returned them.
struct Scored {
    key: String,
    bare: f64,
    prose: f64,
    json: f64,
}

/// This function reads the wazero scores, keyed by fixture.
fn load_scores() -> std::io::Result<Vec<Scored>> {
    let text = std::fs::read_to_string(SCORES_PATH)?;
    let mut out = Vec::new();
    for line in text.lines() {
        let key = field_string(line, "cluster_id");
        let bare = field_number(line, "ours_bare");
        let prose = field_number(line, "ours_prose");
        let json = field_number(line, "ours_json");
        if let (Some(key), Some(bare), Some(prose), Some(json)) = (key, bare, prose, json) {
            out.push(Scored {
                key,
                bare,
                prose,
                json,
            });
        }
    }
    Ok(out)
}

fn score_for<'a>(scores: &'a [Scored], key: &str) -> Option<&'a Scored> {
    scores.iter().find(|row| row.key == key)
}

/// This function prints one public table.
///
/// The SELECTED column is deliberately absent. It names which candidate
/// out of the answer won the match, which is an internal detail of the
/// scorer and is not published. The full diagnostic view keeps it.
fn print_table(scores: &[Scored], fixtures: &[Fixture]) {
    for fixture in fixtures {
        let row = score_for(scores, fixture.key);
        let (bare, prose, json) = match row {
            Some(row) => (row.bare, row.prose, row.json),
            None => (f64::NAN, f64::NAN, f64::NAN),
        };
        let right = selection_is_right(selected(fixture.answer), fixture.wanted_c);
        let worst = bare.max(prose).max(json);
        let flag = if !right && worst >= HONEST_BAR {
            "FARM"
        } else if right {
            "ok"
        } else {
            "miss"
        };
        println!(
            "{:<44} {bare:>8.4} {prose:>8.4} {json:>8.4} {:>3} {flag:>5}",
            truncate(fixture.answer, 44),
            distinct_count(fixture.answer),
        );
    }
}

/// This function prints the column header for a public table.
fn print_header_row() {
    println!(
        "{:<44} {:>8} {:>8} {:>8} {:>3} {:>5}",
        "answer", "bare", "prose", "json", "n", "flag"
    );
}

/// This function prints only the two tables, for a screenshot.
///
/// Every score is the `f32` the compiled module returned under wazero.
/// Nothing here is recomputed. No path and no name reaches the output,
/// and the SELECTED column of the diagnostic view is not printed here.
fn display() -> std::io::Result<()> {
    let scores = load_scores()?;
    println!("ground truth, three renderings:");
    println!("  bare  {TRUTH_BARE}");
    println!("  prose {TRUTH_PROSE}");
    println!("  json  {TRUTH_JSON}");
    println!("honest miner 10% out scores {HONEST_BAR:.4}. Scores are the f32 from wazero.");
    println!();
    println!("CONVERTED ANSWERS FROM A CORRECT MINER");
    print_header_row();
    print_table(&scores, &PLAUSIBLE);
    println!();
    println!("WRONG ANSWERS THAT MENTION 28.1 AS SOME OTHER QUANTITY");
    print_header_row();
    print_table(&scores, &ADVERSARIAL);
    println!();
    println!("COUNTER-ATTACK: GROUND TRUTH VOCABULARY AROUND A WRONG NUMBER");
    print_header_row();
    print_table(&scores, &COUNTER);
    Ok(())
}

// ---------------------------------------------------------------
// the full diagnostic view
// ---------------------------------------------------------------

fn distinct_count(text: &str) -> usize {
    let mut buffer = empty_buffer();
    let count = scan_values(text, &mut buffer);
    let mut seen: Vec<f64> = Vec::new();
    for value in buffer.iter().take(count) {
        if !seen.contains(&value.number) {
            seen.push(value.number);
        }
    }
    seen.len()
}

fn diagnostic(label: &str, fixtures: &[Fixture]) {
    println!();
    println!("=== {label} ===");
    println!(
        "{:<50} {:>9} {:>9} {:>9} {:>3} {:<10} {:>6}",
        "answer text", "bare", "prose", "json", "n", "selected", "flag"
    );
    for fixture in fixtures {
        let bare = score_answer("", TRUTH_BARE, fixture.answer);
        let prose = score_answer("", TRUTH_PROSE, fixture.answer);
        let json = score_answer("", TRUTH_JSON, fixture.answer);
        let pick = selected(fixture.answer);
        let right = selection_is_right(pick, fixture.wanted_c);
        let worst = bare.max(prose).max(json);
        let flag = if !right && worst >= HONEST_BAR {
            "FARM"
        } else if right {
            "ok"
        } else {
            "miss"
        };
        println!(
            "{:<50} {bare:>9.6} {prose:>9.6} {json:>9.6} {:>3} {:<10} {flag:>6}",
            truncate(fixture.answer, 48),
            distinct_count(fixture.answer),
            label_for(pick),
        );
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let outcome = match mode.as_str() {
        "--emit" => emit(),
        "--show" | "--display" => display(),
        _ => {
            println!("ground truth, all three renderings:");
            println!("  bare  {TRUTH_BARE}");
            println!("  prose {TRUTH_PROSE}");
            println!("  json  {TRUTH_JSON}");
            println!();
            println!(
                "honest miner 10 percent out scores {:.6} -- the bar",
                score_answer("", TRUTH_BARE, "30.9 C")
            );
            diagnostic(
                "PLAUSIBLE CONVERTED ANSWERS, correct value 28.1 C",
                &PLAUSIBLE,
            );
            diagnostic("ADVERSARIAL, the answer asserts 34.9 C", &ADVERSARIAL);
            diagnostic(
                "COUNTER-ATTACK, ground truth words around a wrong number",
                &COUNTER,
            );
            Ok(())
        }
    };
    if let Err(error) = outcome {
        eprintln!("prose probe stopped: {error}");
        std::process::exit(1);
    }
}
