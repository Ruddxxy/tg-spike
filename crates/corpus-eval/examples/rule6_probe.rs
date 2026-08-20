//! Which condition tells a quantity intent from a label intent?
//!
//! Dispatch rule 6 returns 0.0 when the ground truth carries a quantity
//! and the answer carries none. It fires on the SHAPE of the truth, so
//! it also fires when the truth's number is metadata nobody asked for:
//! a confidence beside a verdict, a CVSS beside a severity. The correct
//! word then scores 0.0 and the case is dead.
//!
//! Relaxing the rule reopens the farm it exists to close. Measured on
//! the label band, the truth "The temperature was 28.9 C." pays 0.6667
//! for the answer "the temperature was C", which holds no temperature,
//! while an honest miner 10 percent out earns 0.0826.
//!
//! So the rule needs a narrower TRIGGER, not a weaker action.
//!
//! # What this file measures
//!
//! The requirement is on the SCORE, not on the trigger. A trigger that
//! declines to fire is harmless when the text branch pays nothing
//! anyway: the answer "warm and humid today" shares no token with
//! "The temperature was 28.9 C.", so it scores 0.0 with or without the
//! rule. Judging triggers on whether they fire counts that as a farm
//! reopened, which is wrong.
//!
//! So each candidate is scored on what the module would RETURN:
//!
//! ```text
//! predicted = if the candidate fires { 0.0 } else { the text branch }
//! ```
//!
//! The text branch is measured, not modelled: it is what the `label`
//! band returns, since that band is the one that never fires rule 6.
//!
//! Two requirements, and a candidate that misses either is rejected:
//!
//! - a farm case must end at or below the honest bar, 0.0826, the score
//!   an honest miner earns when it is 10 percent out at the weather
//!   band. A non-answer that pays better than honest work is a farm
//!   whatever else it does.
//! - a label pair must separate: the correct word must outscore the
//!   wrong one, and must score above zero.
//!
//! ```text
//! cargo run -p corpus-eval --release --example rule6_probe -- --emit
//! (cd tools/wazero-runner && go run . -golden ../../target/rule6-vectors.json \
//!    -a ../../dist/eval_script_label.wasm -out ../../target/rule6-label.json)
//! cargo run -p corpus-eval --release --example rule6_probe -- --report
//! ```

use eval_script::text::tokenize;
use eval_script::value::{parse_value, scan_truth_values, scan_values, Family, MAX_SCANNED_VALUES};

/// Where `--emit` writes the vectors.
const VECTORS_PATH: &str = "target/rule6-vectors.json";
/// Where the label band's scores are read from.
const LABEL_SCORES_PATH: &str = "target/rule6-label.json";

/// The honest-miner bar: what a correct miner earns when it is 10
/// percent out at the weather band. A farm answer must not reach it.
const HONEST_BAR: f64 = 0.0826;

/// A zero value, for scan buffers.
const ZERO: eval_script::value::ParsedValue = eval_script::value::ParsedValue {
    number: 0.0,
    unit: eval_script::value::Unit::None,
};

/// One farm case: a quantity truth and an answer that supplies no
/// quantity. Whatever the trigger does, the score must stay at or
/// below the honest bar.
struct Farm {
    /// A short name for the table.
    name: &'static str,
    /// The question, exactly as the corpus carries it.
    question: &'static str,
    /// The ground truth text.
    truth: &'static str,
    /// The answer that supplies no quantity.
    answer: &'static str,
}

/// One label case: a truth whose number is metadata, with a correct
/// word and a wrong word. The correct word must win.
struct Pair {
    /// A short name for the table.
    name: &'static str,
    /// The question, exactly as the promotion set carries it.
    question: &'static str,
    /// The ground truth text.
    truth: &'static str,
    /// The word a correct miner gives.
    good: &'static str,
    /// The word a wrong miner gives.
    bad: &'static str,
}

/// The farm side.
///
/// The questions are the ones the weather corpus really carries,
/// including `[direct] 18 -> /predict`, which holds no words, and
/// "Will Riyadh issue heat warnings?", which reads as a yes/no
/// question and is not one. Both are common: the first covers 722 of
/// the 6,169 rows and the second family covers about 1,780 more.
/// The last three are ADVERSARIAL: they are built to land the echo
/// measure inside the range the label cases occupy. If a farm can be
/// built at any echo a label case reaches, then no threshold on that
/// measure separates the two, and the overlap is a property of the
/// measure rather than of the cases that were picked.
const FARMS: [Farm; 20] = [
    Farm {
        name: "prose/full-echo",
        question: "[direct] 18 -> /predict",
        truth: "The temperature was 28.9 C.",
        answer: "the temperature was C",
    },
    Farm {
        name: "prose/echo-punct",
        question: "[direct] 18 -> /predict",
        truth: "The temperature was 28.9 C.",
        answer: "The temperature was C.",
    },
    Farm {
        name: "prose/half-echo",
        question: "What is the current weather in Tokyo, Japan?",
        truth: "The temperature was 28.9 C.",
        answer: "the temperature",
    },
    Farm {
        name: "prose/one-word",
        question: "What is the current weather in Tokyo, Japan?",
        truth: "The temperature was 28.9 C.",
        answer: "temperature",
    },
    Farm {
        name: "prose/unit-only",
        question: "What's the 3-day forecast for Miami, FL?",
        truth: "The temperature was 28.9 C.",
        answer: "C",
    },
    Farm {
        name: "prose/yes-no-q",
        question: "Will Riyadh issue heat warnings?",
        truth: "The temperature was 45.2 C.",
        answer: "the temperature was C",
    },
    Farm {
        name: "prose/unrelated",
        question: "What is the current weather in Tokyo, Japan?",
        truth: "The temperature was 28.9 C.",
        answer: "warm and humid today",
    },
    Farm {
        name: "json/key-echo",
        question: "What is the current weather in Tokyo, Japan?",
        truth: "{\"temperature_2m\":28.9,\"time\":\"2026-08-10T12:00\"}",
        answer: "temperature and time",
    },
    Farm {
        name: "json/one-word",
        question: "[direct] 18 -> /predict",
        truth: "{\"temperature_2m\":28.9,\"time\":\"2026-08-10T12:00\"}",
        answer: "temperature",
    },
    Farm {
        name: "bare/unrelated",
        question: "[direct] 18 -> /predict",
        truth: "28.9",
        answer: "warm",
    },
    Farm {
        name: "price/scaffold",
        question: "[direct] 207 -> /price",
        truth: "The price is 192.43 USD.",
        answer: "the price is USD",
    },
    Farm {
        name: "gas/unit-only",
        question: "[direct] 219 -> /gas",
        truth: "12 gwei",
        answer: "gwei",
    },
    Farm {
        name: "weather/q37-echo",
        question: "[direct] 211 -> /weather",
        truth: "The temperature is 28.9 C.",
        answer: "the temperature is C",
    },
    // A truth with three non-numeric tokens puts one token at an echo
    // of 1/3, which is where the label cases sit.
    Farm {
        name: "adv/echo-third",
        question: "[direct] 18 -> /predict",
        truth: "Temperature was 28.9 C",
        answer: "temperature",
    },
    // Five non-numeric tokens puts two tokens at 2/5, and four puts
    // one at 1/4. Between them they cover the whole label range.
    Farm {
        name: "adv/echo-two-fifths",
        question: "[direct] 18 -> /predict",
        truth: "The reported temperature was 28.9 degrees C",
        answer: "reported temperature",
    },
    Farm {
        name: "adv/echo-two-sevenths",
        question: "[direct] 18 -> /predict",
        truth: "The station reported that the temperature was 28.9 C",
        answer: "station reported",
    },
    // The quoted-value exemption asks whether the answer is a string
    // the truth carries. A quantity truth that names its city carries
    // one, and the city is not the temperature. The corpus rendering
    // happens to put a timestamp there, and a timestamp holds digits
    // so it never reaches rule 6, but nothing in the ABI promises the
    // rendering. This case is the exemption used against itself.
    Farm {
        name: "adv/quoted-city",
        question: "What is the current weather in Tokyo, Japan?",
        truth: "{\"temperature_2m\":28.9,\"city\":\"Tokyo\"}",
        answer: "Tokyo",
    },
    // CEILING PROBES.
    //
    // An attenuated rule pays a fraction of the text branch, so what
    // the rule can ever pay is that fraction times the LARGEST score
    // the text branch gives a no-quantity answer. These three find
    // that largest score. The answer is the whole truth with the
    // number taken out, so recall of the truth's vocabulary is total
    // and only the missing number separates the two texts. A longer
    // truth makes the one missing token a smaller share, so the score
    // climbs towards 1.0 as the truth grows.
    Farm {
        name: "ceiling/5-token",
        question: "[direct] 18 -> /predict",
        truth: "The temperature was 28.9 C.",
        answer: "the temperature was C",
    },
    Farm {
        name: "ceiling/12-token",
        question: "[direct] 18 -> /predict",
        truth: "The station at the site reported that the outdoor air temperature was 28.9 C",
        answer: "the station at site reported that outdoor air temperature was C",
    },
    Farm {
        name: "ceiling/23-token",
        question: "[direct] 18 -> /predict",
        truth: "The weather station on the north side of the airport reported that the \
                outdoor air temperature measured at two metres above ground level was 28.9 C",
        answer: "The weather station on the north side of the airport reported that the \
                 outdoor air temperature measured at two metres above ground level was C",
    },
];

/// One honest reference: a miner that gives a real number and is out by
/// a stated amount.
///
/// An attenuated rule is safe only if what it pays stays under what an
/// honest miner earns, and that figure is not one number. It is the
/// numeric curve, so it differs per band: at the weather band a miner
/// 10 percent out earns 0.0826, and at the price band the same 10
/// percent earns 0.0004 because the price band is calibrated for a
/// tighter answer. These rows measure that bar on each band instead of
/// assuming one figure covers all four.
struct Honest {
    /// A short name for the table.
    name: &'static str,
    /// The question text.
    question: &'static str,
    /// The ground truth text.
    truth: &'static str,
    /// The answer a real but imperfect miner gives.
    answer: &'static str,
    /// How far out the answer is.
    note: &'static str,
}

/// The honest references.
const HONEST: [Honest; 6] = [
    Honest {
        name: "temp/10pct",
        question: "[direct] 211 -> /weather",
        truth: "The temperature was 28.9 C.",
        answer: "31.79 C",
        note: "10 percent out on a temperature",
    },
    Honest {
        name: "temp/3pct",
        question: "[direct] 211 -> /weather",
        truth: "The temperature was 28.9 C.",
        answer: "29.77 C",
        note: "3 percent out on a temperature",
    },
    Honest {
        name: "price/10pct",
        question: "[direct] 207 -> /price",
        truth: "The price is 192.43 USD.",
        answer: "211.67 USD",
        note: "10 percent out on a price",
    },
    Honest {
        name: "price/1pct",
        question: "[direct] 207 -> /price",
        truth: "The price is 192.43 USD.",
        answer: "194.35 USD",
        note: "1 percent out on a price",
    },
    Honest {
        name: "price/02pct",
        question: "[direct] 207 -> /price",
        truth: "The price is 192.43 USD.",
        answer: "192.82 USD",
        note: "0.2 percent out on a price",
    },
    Honest {
        name: "gas/10pct",
        question: "[direct] 219 -> /gas",
        truth: "12 gwei",
        answer: "13.2 gwei",
        note: "10 percent out on a gas price",
    },
];

/// The bands an attenuated rule has to be safe on.
const BANDS: [&str; 4] = ["weather", "price", "onchain", "label"];

/// The label side: the six the promotion set loses, as pairs.
const PAIRS: [Pair; 6] = [
    Pair {
        name: "q02/URL_SCAN/json",
        question: "Scan http://login-secure-update.tld/",
        truth: "{\"verdict\":\"phishing\",\"confidence\":0.97}",
        good: "phishing",
        bad: "benign",
    },
    Pair {
        name: "q09/CVE/prose",
        question: "Severity of CVE-2021-44228",
        truth: "CVE-2021-44228 has a severity rating of CRITICAL.",
        good: "CRITICAL",
        bad: "MEDIUM",
    },
    Pair {
        name: "q10/CVE/json",
        question: "Severity of CVE-2021-44228",
        truth: "{\"cve\":\"CVE-2021-44228\",\"severity\":\"critical\",\"cvss\":9.8}",
        good: "critical",
        bad: "low",
    },
    Pair {
        name: "q14/SENTIMENT/json",
        question: "Sentiment of: the delivery was late and the box was open",
        truth: "{\"label\":\"negative\",\"score\":0.88}",
        good: "negative",
        bad: "positive",
    },
    Pair {
        name: "q22/FACT_CHECK/prose",
        question: "Claim: the programme cut transmission by half.",
        truth: "Partly true. The programme reduced transmission by 40%.",
        good: "partly true",
        bad: "60%",
    },
    Pair {
        name: "q23/FACT_CHECK/json",
        question: "Claim: the Great Wall is visible from the Moon.",
        truth: "{\"verdict\":\"false\",\"sources\":3}",
        good: "false",
        bad: "true",
    },
];

// -----------------------------------------------------------------
// The candidates
// -----------------------------------------------------------------

/// Words that name a quantity, for candidate 1.
///
/// The list is generous on purpose. A candidate that fails with a
/// generous list fails with a careful one.
const QUANTITY_WORDS: [&str; 22] = [
    "temperature",
    "temp",
    "price",
    "cost",
    "rate",
    "how",
    "much",
    "many",
    "hot",
    "cold",
    "warm",
    "degrees",
    "celsius",
    "fahrenheit",
    "gas",
    "fee",
    "forecast",
    "weather",
    "predict",
    "value",
    "amount",
    "number",
];

/// Candidate 1: the QUESTION names a quantity, or holds a digit.
fn c1_question_names_a_quantity(question: &str, _truth: &str, _answer: &str) -> bool {
    let tokens = tokenize(question);
    for token in tokens.tokens() {
        if QUANTITY_WORDS.contains(&token.as_str()) {
            return true;
        }
    }
    question.chars().any(|character| character.is_ascii_digit())
}

/// Candidate 2: the truth's numeric field KEY appears in the question.
///
/// A truth with no `"key":number` pair has no key to look for. The
/// rule then has to choose a default, and the safe default is to fire,
/// because that is what the module does today.
fn c2_key_in_question(question: &str, truth: &str, _answer: &str) -> bool {
    let keys = numeric_field_keys(truth);
    if keys.is_empty() {
        return true;
    }
    let lowered = lowercase(question);
    keys.iter().any(|key| lowered.contains(&lowercase(key)))
}

/// Candidate 3: the truth carries no OTHER candidate answer.
///
/// A quoted string that is a value rather than a key is a word the
/// intent might be asking for. A truth that carries one is a truth
/// whose number may be metadata.
fn c3_quantity_is_the_only_content(_question: &str, truth: &str, _answer: &str) -> bool {
    quoted_values(truth).is_empty()
}

/// Candidate 5: fire unless the answer IS one of the truth's quoted
/// values.
fn c5_not_a_quoted_value(_question: &str, truth: &str, answer: &str) -> bool {
    !answer_is_a_quoted_value(truth, answer)
}

/// Candidate 6: the truth's quantity carries a unit.
fn c6_quantity_has_a_unit(_question: &str, truth: &str, _answer: &str) -> bool {
    let mut values = [ZERO; MAX_SCANNED_VALUES];
    let count = scan_truth_values(truth, &mut values);
    values[..count]
        .iter()
        .any(|value| value.family() != Family::Dimensionless)
}

/// This function measures how much of the truth's vocabulary the
/// answer gives back.
///
/// The farm's whole method is to restate the words around the number.
/// The divisor is the TRUTH's token count, which the miner does not
/// choose, so padding the answer cannot raise it.
fn echo_recall(truth: &str, answer: &str) -> f64 {
    let truth_tokens = tokenize(truth);
    let answer_tokens = tokenize(answer);
    let mut wanted = 0usize;
    let mut given = 0usize;
    for token in truth_tokens.tokens() {
        // A token that reads as a number is not vocabulary. The answer
        // has none by construction, so counting them would push every
        // recall down by the same amount and blur the two shapes.
        if parse_value(token.as_str()).is_some() {
            continue;
        }
        wanted += 1;
        if answer_tokens.contains(token) {
            given += 1;
        }
    }
    if wanted == 0 {
        return 0.0;
    }
    (given as f64) / (wanted as f64)
}

/// Candidate 4 at each threshold.
fn c4_echo(threshold: f64) -> impl Fn(&str, &str, &str) -> bool {
    move |_question: &str, truth: &str, answer: &str| echo_recall(truth, answer) >= threshold
}

/// Candidate 7: the echo test, with the quoted-value exemption in
/// front of it.
fn c7_echo_unless_quoted(threshold: f64) -> impl Fn(&str, &str, &str) -> bool {
    move |_question: &str, truth: &str, answer: &str| {
        !answer_is_a_quoted_value(truth, answer) && echo_recall(truth, answer) >= threshold
    }
}

// -----------------------------------------------------------------
// Small JSON helpers
// -----------------------------------------------------------------

/// This function lowercases a text, ASCII only.
fn lowercase(text: &str) -> String {
    text.chars()
        .map(eval_script::text::to_ascii_lowercase_char)
        .collect()
}

/// This function tells if the answer equals one of the truth's quoted
/// values.
fn answer_is_a_quoted_value(truth: &str, answer: &str) -> bool {
    let trimmed = lowercase(answer.trim());
    quoted_values(truth)
        .iter()
        .any(|value| lowercase(value) == trimmed)
}

/// This function reads the quoted spans of a text and says which are
/// keys and which are values.
///
/// A span followed by a colon is a key. Everything else is a value.
/// This is a reader for the shape the corpus really carries, not a
/// JSON parser.
fn quoted_spans(text: &str) -> Vec<(String, bool)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'"' {
            end += 1;
        }
        if end >= bytes.len() {
            break;
        }
        let mut after = end + 1;
        while after < bytes.len() && bytes[after] == b' ' {
            after += 1;
        }
        let is_key = after < bytes.len() && bytes[after] == b':';
        out.push((text[start..end].to_string(), is_key));
        index = end + 1;
    }
    out
}

/// This function gives the quoted VALUES of a text.
fn quoted_values(text: &str) -> Vec<String> {
    quoted_spans(text)
        .into_iter()
        .filter(|(_, is_key)| !is_key)
        .map(|(span, _)| span)
        .collect()
}

/// This function gives the keys whose value is a bare number.
fn numeric_field_keys(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for (span, is_key) in quoted_spans(text) {
        if !is_key {
            continue;
        }
        let needle = format!("\"{span}\"");
        let Some(at) = text.find(&needle) else {
            continue;
        };
        let mut cursor = at + needle.len();
        while cursor < bytes.len() && (bytes[cursor] == b' ' || bytes[cursor] == b':') {
            cursor += 1;
        }
        if cursor < bytes.len() && (bytes[cursor].is_ascii_digit() || bytes[cursor] == b'-') {
            out.push(span);
        }
    }
    out
}

// -----------------------------------------------------------------
// --emit
// -----------------------------------------------------------------

/// This function writes every case in the golden-vector shape.
fn emit() -> std::io::Result<()> {
    let mut vectors = Vec::new();
    for (index, farm) in FARMS.iter().enumerate() {
        vectors.push(serde_json::json!({
            "name": format!("farm{index:02}"),
            "question": farm.question,
            "ground_truth": farm.truth,
            "miner_answer": farm.answer,
            "expected": 0.0,
        }));
    }
    for (index, pair) in PAIRS.iter().enumerate() {
        for (suffix, answer) in [("good", pair.good), ("bad", pair.bad)] {
            vectors.push(serde_json::json!({
                "name": format!("pair{index:02}-{suffix}"),
                "question": pair.question,
                "ground_truth": pair.truth,
                "miner_answer": answer,
                "expected": 0.0,
            }));
        }
    }
    for (index, honest) in HONEST.iter().enumerate() {
        vectors.push(serde_json::json!({
            "name": format!("honest{index:02}"),
            "question": honest.question,
            "ground_truth": honest.truth,
            "miner_answer": honest.answer,
            "expected": 0.0,
        }));
    }
    let document = serde_json::json!({ "vectors": vectors });
    std::fs::write(
        VECTORS_PATH,
        format!("{}\n", serde_json::to_string_pretty(&document)?),
    )?;
    println!("wrote {} vectors to {VECTORS_PATH}", vectors.len());
    println!();
    println!("Score them with EVERY band. The label band's score is the text");
    println!("branch, because that band never fires rule 6. The other three give");
    println!("the honest bar, which is a different number on each band.");
    println!();
    for band in BANDS {
        let flags = if band == "weather" {
            String::new()
        } else {
            format!(" --no-default-features --features {band}")
        };
        println!("  cargo build -p eval-script --release --target wasm32-unknown-unknown{flags}");
        println!("  (cd tools/wazero-runner && go run . -golden ../../{VECTORS_PATH} \\");
        println!("     -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \\");
        println!("     -out ../../target/rule6-{band}.json)");
    }
    Ok(())
}

/// This function reads a score file written by the engine runner.
fn load_scores(path: &str) -> std::io::Result<std::collections::HashMap<String, f64>> {
    let text = std::fs::read_to_string(path)?;
    let document: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut table = std::collections::HashMap::new();
    for row in document["vectors"].as_array().into_iter().flatten() {
        if let (Some(name), Some(value)) = (row["name"].as_str(), row["value"].as_f64()) {
            table.insert(name.to_string(), value);
        }
    }
    Ok(table)
}

// -----------------------------------------------------------------
// --report
// -----------------------------------------------------------------

/// How a candidate did.
struct Tally {
    /// Farm cases whose predicted score reaches the honest bar.
    farms_open: Vec<(&'static str, f64)>,
    /// Label pairs the candidate still fails to separate.
    dead_pairs: Vec<&'static str>,
}

impl Tally {
    /// This function prints one candidate's result.
    fn print(&self, name: &str) {
        let verdict = if self.farms_open.is_empty() && self.dead_pairs.is_empty() {
            "SEPARATES"
        } else if !self.farms_open.is_empty() {
            "REJECTED, reopens a farm"
        } else {
            "insufficient, cases stay dead"
        };
        println!(
            "   {:<52} {:>2} farm  {:>1} dead   {verdict}",
            name,
            self.farms_open.len(),
            self.dead_pairs.len()
        );
        for (case, score) in &self.farms_open {
            println!("        farm {case} would pay {score:.4}, the honest bar is {HONEST_BAR:.4}");
        }
        if !self.dead_pairs.is_empty() {
            println!("        still dead: {}", self.dead_pairs.join(" "));
        }
    }
}

/// This function scores one candidate against every case.
fn judge(
    fires: &dyn Fn(&str, &str, &str) -> bool,
    label: &std::collections::HashMap<String, f64>,
) -> Tally {
    let mut farms_open = Vec::new();
    let mut dead_pairs = Vec::new();

    for (index, farm) in FARMS.iter().enumerate() {
        let text_branch = *label.get(&format!("farm{index:02}")).unwrap_or(&-1.0);
        let predicted = if fires(farm.question, farm.truth, farm.answer) {
            0.0
        } else {
            text_branch
        };
        if predicted >= HONEST_BAR {
            farms_open.push((farm.name, predicted));
        }
    }

    for (index, pair) in PAIRS.iter().enumerate() {
        let good_text = *label.get(&format!("pair{index:02}-good")).unwrap_or(&-1.0);
        let bad_text = *label.get(&format!("pair{index:02}-bad")).unwrap_or(&-1.0);
        let good = if fires(pair.question, pair.truth, pair.good) {
            0.0
        } else {
            good_text
        };
        let bad = if fires(pair.question, pair.truth, pair.bad) {
            0.0
        } else {
            bad_text
        };
        if !(good > bad && good > 0.0) {
            dead_pairs.push(pair.name);
        }
    }

    Tally {
        farms_open,
        dead_pairs,
    }
}

/// The echo thresholds tried.
const ECHO_THRESHOLDS: [f64; 6] = [0.20, 0.25, 0.30, 0.34, 0.40, 0.50];

fn report() -> std::io::Result<()> {
    let label = load_scores(LABEL_SCORES_PATH)?;

    println!("=== CANDIDATE TRIGGERS FOR DISPATCH RULE 6 ===");
    println!();
    println!("predicted score = if the candidate fires {{ 0.0 }} else {{ the text branch }}");
    println!("the text branch is the label band's measured score, not a model.");
    println!("a farm must stay below the honest bar {HONEST_BAR:.4}.");
    println!();

    // Every case must really reach rule 6, or it proves nothing.
    let mut unreachable = Vec::new();
    let mut check = |name: &'static str, truth: &str, answer: &str| {
        let mut truth_values = [ZERO; MAX_SCANNED_VALUES];
        let truth_count = scan_truth_values(truth, &mut truth_values);
        let mut answer_values = [ZERO; MAX_SCANNED_VALUES];
        let answer_count = scan_values(answer, &mut answer_values);
        if truth_count == 0 || answer_count != 0 {
            unreachable.push((name, truth_count, answer_count));
        }
    };
    for farm in FARMS.iter() {
        check(farm.name, farm.truth, farm.answer);
    }
    for pair in PAIRS.iter() {
        check(pair.name, pair.truth, pair.good);
        check(pair.name, pair.truth, pair.bad);
    }
    if unreachable.is_empty() {
        println!("all {} cases reach rule 6", FARMS.len() + PAIRS.len() * 2);
    } else {
        println!("THESE CASES DO NOT REACH RULE 6, so they prove nothing here:");
        for (name, truth_count, answer_count) in &unreachable {
            println!("   {name:<24} truth values {truth_count}, answer values {answer_count}");
        }
    }
    println!();

    println!("1. what the text branch pays, measured on the label band");
    println!("   {:<24} {:>9}  answer", "farm case", "text");
    for (index, farm) in FARMS.iter().enumerate() {
        let text_branch = *label.get(&format!("farm{index:02}")).unwrap_or(&-1.0);
        let flag = if text_branch >= HONEST_BAR {
            "  <-- above the honest bar"
        } else {
            ""
        };
        println!(
            "   {:<24} {:>9.4}  {:?}{}",
            farm.name, text_branch, farm.answer, flag
        );
    }
    println!();
    println!(
        "   {:<24} {:>9} {:>9}  echo good/bad",
        "label pair", "good", "bad"
    );
    for (index, pair) in PAIRS.iter().enumerate() {
        println!(
            "   {:<24} {:>9.4} {:>9.4}  {:.2}/{:.2}",
            pair.name,
            *label.get(&format!("pair{index:02}-good")).unwrap_or(&-1.0),
            *label.get(&format!("pair{index:02}-bad")).unwrap_or(&-1.0),
            echo_recall(pair.truth, pair.good),
            echo_recall(pair.truth, pair.bad),
        );
    }
    println!();

    println!("2. every candidate");
    println!();
    judge(&c1_question_names_a_quantity, &label)
        .print("c1  question names a quantity or holds a digit");
    judge(&c2_key_in_question, &label)
        .print("c2  truth's numeric field key appears in the question");
    judge(&c3_quantity_is_the_only_content, &label)
        .print("c3  truth carries no other candidate answer");
    judge(&c5_not_a_quoted_value, &label).print("c5  fire unless the answer is a quoted value");
    judge(&c6_quantity_has_a_unit, &label).print("c6  the truth's quantity carries a unit");
    println!();
    for threshold in ECHO_THRESHOLDS {
        judge(&c4_echo(threshold), &label).print(&format!(
            "c4  answer gives back >= {threshold:.2} of the truth"
        ));
    }
    println!();
    for threshold in ECHO_THRESHOLDS {
        judge(&c7_echo_unless_quoted(threshold), &label).print(&format!(
            "c7  echo >= {threshold:.2}, unless the answer is a quoted value"
        ));
    }
    println!();

    println!("3. the measure, sorted");
    let mut farm_echo: Vec<(f64, &str)> = FARMS
        .iter()
        .map(|farm| (echo_recall(farm.truth, farm.answer), farm.name))
        .collect();
    let mut pair_echo: Vec<(f64, &str)> = PAIRS
        .iter()
        .flat_map(|pair| {
            [
                (echo_recall(pair.truth, pair.good), pair.name),
                (echo_recall(pair.truth, pair.bad), pair.name),
            ]
        })
        .collect();
    farm_echo.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    pair_echo.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    println!(
        "   farm echo  {:.2?}",
        farm_echo.iter().map(|v| v.0).collect::<Vec<_>>()
    );
    println!(
        "   label echo {:.2?}",
        pair_echo.iter().map(|v| v.0).collect::<Vec<_>>()
    );
    Ok(())
}

// -----------------------------------------------------------------
// --attenuate
// -----------------------------------------------------------------

/// This function gives the evidence, in [0,1], that a quantity was the
/// wanted answer.
///
/// It combines the signals the seven triggers used one at a time. None
/// of them carried enough alone, so each contributes a share rather
/// than a verdict. The quoted-value signal points the other way, so it
/// subtracts.
fn quantity_evidence(question: &str, truth: &str, answer: &str) -> f64 {
    let mut evidence = 0.0;
    if c1_question_names_a_quantity(question, truth, answer) {
        evidence += 0.25;
    }
    if c3_quantity_is_the_only_content(question, truth, answer) {
        evidence += 0.25;
    }
    if c6_quantity_has_a_unit(question, truth, answer) {
        evidence += 0.25;
    }
    evidence += 0.25 * echo_recall(truth, answer);
    if answer_is_a_quoted_value(truth, answer) {
        evidence -= 0.50;
    }
    evidence.clamp(0.0, 1.0)
}

/// This function tells if a text carries a quantity that `scan_values`
/// finds.
///
/// An answer that carries one never reaches rule 6. Its score comes
/// from the numeric branch and attenuation cannot change it.
fn holds_a_value(text: &str) -> bool {
    let mut values = [ZERO; MAX_SCANNED_VALUES];
    scan_values(text, &mut values) != 0
}

/// This function reports whether an attenuated rule 6 can work.
fn attenuate() -> std::io::Result<()> {
    let label = load_scores(LABEL_SCORES_PATH)?;
    let mut bands = std::collections::HashMap::new();
    for band in BANDS {
        bands.insert(band, load_scores(&format!("target/rule6-{band}.json"))?);
    }

    println!("=== AN ATTENUATED RULE 6 ===");
    println!();
    println!("Rule 6 returns 0.0 today. An attenuated rule returns");
    println!();
    println!("    alpha * the text branch");
    println!();
    println!("so a farm and a correct label answer both pay a cost rather than");
    println!("one of them being killed outright. An answer that carries a value");
    println!("never reaches rule 6, so its score is the band's numeric score and");
    println!("no alpha changes it.");
    println!();

    // The ceiling. What the text branch pays a no-quantity answer at
    // its very best decides what alpha can ever be.
    println!("1. the ceiling: the most the text branch pays a no-quantity answer");
    let mut ceiling = 0.0f64;
    let mut ceiling_name = "";
    for (index, farm) in FARMS.iter().enumerate() {
        let text = *label.get(&format!("farm{index:02}")).unwrap_or(&-1.0);
        if text > ceiling {
            ceiling = text;
            ceiling_name = farm.name;
        }
        if farm.name.starts_with("ceiling/") {
            println!("   {:<24} {:>9.4}", farm.name, text);
        }
    }
    println!(
        "   {:<24} {:>9.4}  <- the largest of all {} farm rows, {ceiling_name}",
        "MEASURED CEILING",
        ceiling,
        FARMS.len()
    );
    println!("   A longer truth pushes this towards 1.0, so treat it as a floor");
    println!("   on the true ceiling, not the true ceiling.");
    println!();

    // The honest bar, per band.
    println!("2. the honest bar, per band");
    println!("   what a miner earns when it gives a REAL number and is out by:");
    print!("   {:<20}", "case");
    for band in BANDS {
        print!(" {band:>10}");
    }
    println!("  note");
    for (index, honest) in HONEST.iter().enumerate() {
        print!("   {:<20}", honest.name);
        for band in BANDS {
            let value = *bands[band]
                .get(&format!("honest{index:02}"))
                .unwrap_or(&-1.0);
            print!(" {value:>10.6}");
        }
        println!("  {}", honest.note);
    }
    println!();

    // The window for a CONSTANT alpha, per band.
    println!("3. the window for a constant alpha");
    println!("   alpha must be small enough that no farm reaches the honest bar,");
    println!("   and large enough that a correct label answer still beats a wrong");
    println!("   one. A band whose window is empty cannot use a constant alpha.");
    println!();
    println!(
        "   {:<9} {:>12} {:>12} {:>12}  window",
        "band", "bar", "alpha <", "alpha >"
    );
    for band in BANDS {
        // The bar for this band is the weakest honest answer that
        // should still outrank a non-answer. The 10 percent row is the
        // one the repository already uses for the weather band.
        let bar_key = match band {
            "price" => "honest02", // price/10pct
            "onchain" => "honest05",
            _ => "honest00",
        };
        let bar = *bands[band].get(bar_key).unwrap_or(&-1.0);

        // alpha must satisfy alpha * text < bar for every farm.
        let mut alpha_max = f64::INFINITY;
        for (index, _farm) in FARMS.iter().enumerate() {
            let text = *label.get(&format!("farm{index:02}")).unwrap_or(&0.0);
            if text > 0.0 {
                alpha_max = alpha_max.min(bar / text);
            }
        }

        // alpha must satisfy alpha * good_text > bad, for every pair
        // whose bad answer is scored by the numeric branch. A bad
        // answer with no value is itself attenuated, so it cancels.
        let mut alpha_min: f64 = 0.0;
        for (index, pair) in PAIRS.iter().enumerate() {
            let good_text = *label.get(&format!("pair{index:02}-good")).unwrap_or(&0.0);
            if good_text <= 0.0 {
                continue;
            }
            if holds_a_value(pair.bad) {
                let bad = *bands[band]
                    .get(&format!("pair{index:02}-bad"))
                    .unwrap_or(&0.0);
                alpha_min = alpha_min.max(bad / good_text);
            }
        }

        let open = alpha_min < alpha_max;
        println!(
            "   {:<9} {:>12.6} {:>12.6} {:>12.6}  {}",
            band,
            bar,
            alpha_max,
            alpha_min,
            if open { "OPEN" } else { "EMPTY" }
        );
    }
    println!();

    // The signal-combined alpha.
    println!("4. a signal-combined alpha");
    println!("   alpha = A * (1 - evidence), evidence from the same signals the");
    println!("   seven triggers used, combined rather than thresholded.");
    println!();
    println!(
        "   {:<24} {:>9} {:>9} {:>9}",
        "case", "text", "evidence", "1-evid"
    );
    for (index, farm) in FARMS.iter().enumerate() {
        let text = *label.get(&format!("farm{index:02}")).unwrap_or(&-1.0);
        let evidence = quantity_evidence(farm.question, farm.truth, farm.answer);
        println!(
            "   {:<24} {:>9.4} {:>9.4} {:>9.4}",
            farm.name,
            text,
            evidence,
            1.0 - evidence
        );
    }
    for (index, pair) in PAIRS.iter().enumerate() {
        let text = *label.get(&format!("pair{index:02}-good")).unwrap_or(&-1.0);
        let evidence = quantity_evidence(pair.question, pair.truth, pair.good);
        println!(
            "   {:<24} {:>9.4} {:>9.4} {:>9.4}  (label good)",
            pair.name,
            text,
            evidence,
            1.0 - evidence
        );
    }
    println!();

    // The same window question, now with the per-case factor folded in.
    println!("5. the window for the signal-combined alpha");
    println!(
        "   {:<9} {:>12} {:>12} {:>12}  window",
        "band", "bar", "A <", "A >"
    );
    for band in BANDS {
        let bar_key = match band {
            "price" => "honest02",
            "onchain" => "honest05",
            _ => "honest00",
        };
        let bar = *bands[band].get(bar_key).unwrap_or(&-1.0);

        let mut a_max = f64::INFINITY;
        for (index, farm) in FARMS.iter().enumerate() {
            let text = *label.get(&format!("farm{index:02}")).unwrap_or(&0.0);
            let factor = 1.0 - quantity_evidence(farm.question, farm.truth, farm.answer);
            let effective = text * factor;
            if effective > 0.0 {
                a_max = a_max.min(bar / effective);
            }
        }

        let mut a_min: f64 = 0.0;
        for (index, pair) in PAIRS.iter().enumerate() {
            let good_text = *label.get(&format!("pair{index:02}-good")).unwrap_or(&0.0);
            let factor = 1.0 - quantity_evidence(pair.question, pair.truth, pair.good);
            let effective = good_text * factor;
            if effective <= 0.0 {
                continue;
            }
            if holds_a_value(pair.bad) {
                let bad = *bands[band]
                    .get(&format!("pair{index:02}-bad"))
                    .unwrap_or(&0.0);
                a_min = a_min.max(bad / effective);
            }
        }

        let open = a_min < a_max;
        println!(
            "   {:<9} {:>12.6} {:>12.6} {:>12.6}  {}",
            band,
            bar,
            a_max,
            a_min,
            if open { "OPEN" } else { "EMPTY" }
        );
    }
    println!();

    // The distributions, sorted, the way the echo analysis reported.
    println!("6. the two distributions, sorted");
    println!("   the text branch pays these, before any alpha:");
    let mut farm_text: Vec<f64> = (0..FARMS.len())
        .map(|index| *label.get(&format!("farm{index:02}")).unwrap_or(&-1.0))
        .collect();
    let mut good_text: Vec<f64> = (0..PAIRS.len())
        .map(|index| *label.get(&format!("pair{index:02}-good")).unwrap_or(&-1.0))
        .collect();
    farm_text.sort_by(|a, b| a.partial_cmp(b).unwrap());
    good_text.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("   farm  {farm_text:.4?}");
    println!("   label {good_text:.4?}");
    println!();
    println!("   A constant alpha scales both lists by the same number, so it can");
    println!("   never move one past the other. What decides the question is not");
    println!("   whether the lists overlap but whether the WHOLE farm list can be");
    println!("   pushed under the bar while the label list stays above its own.");
    println!();

    // The window depends on which honest miner the farm has to lose to.
    // Section 3 used the 10 percent row because that is the figure the
    // repository already quotes. That choice is an argument, not a
    // measurement, so here is the same window against every reference.
    println!("7. sensitivity: the window against each honest reference");
    println!("   the bar is an argument about how bad an honest miner may be, so");
    println!("   the window is recomputed against each one. A band that is open");
    println!("   against a loose bar and empty against a strict one is open only");
    println!("   for as long as that argument holds.");
    println!();
    println!(
        "   {:<9} {:<12} {:>12} {:>12} {:>12}  window",
        "band", "reference", "bar", "alpha <", "alpha >"
    );
    for band in BANDS {
        for (index, honest) in HONEST.iter().enumerate() {
            let bar = *bands[band]
                .get(&format!("honest{index:02}"))
                .unwrap_or(&-1.0);
            let mut alpha_max = f64::INFINITY;
            for (farm_index, _farm) in FARMS.iter().enumerate() {
                let text = *label.get(&format!("farm{farm_index:02}")).unwrap_or(&0.0);
                if text > 0.0 {
                    alpha_max = alpha_max.min(bar / text);
                }
            }
            let mut alpha_min: f64 = 0.0;
            for (pair_index, pair) in PAIRS.iter().enumerate() {
                let good_text = *label
                    .get(&format!("pair{pair_index:02}-good"))
                    .unwrap_or(&0.0);
                if good_text <= 0.0 || !holds_a_value(pair.bad) {
                    continue;
                }
                let bad = *bands[band]
                    .get(&format!("pair{pair_index:02}-bad"))
                    .unwrap_or(&0.0);
                alpha_min = alpha_min.max(bad / good_text);
            }
            println!(
                "   {:<9} {:<12} {:>12.6} {:>12.6} {:>12.6}  {}",
                band,
                honest.name,
                bar,
                alpha_max,
                alpha_min,
                if alpha_min < alpha_max {
                    "OPEN"
                } else {
                    "EMPTY"
                }
            );
        }
    }
    println!();

    // A concrete alpha per band, and what every case then scores.
    println!("8. a concrete alpha per band, and every case under it");
    println!("   alpha is the geometric mean of the section 3 window, so it sits");
    println!("   as far from both walls as the window allows.");
    println!();
    for band in BANDS {
        let bar_key = match band {
            "price" => "honest02",
            "onchain" => "honest05",
            _ => "honest00",
        };
        let bar = *bands[band].get(bar_key).unwrap_or(&-1.0);
        let mut alpha_max = f64::INFINITY;
        for (index, _farm) in FARMS.iter().enumerate() {
            let text = *label.get(&format!("farm{index:02}")).unwrap_or(&0.0);
            if text > 0.0 {
                alpha_max = alpha_max.min(bar / text);
            }
        }
        let mut alpha_min: f64 = 0.0;
        for (index, pair) in PAIRS.iter().enumerate() {
            let good_text = *label.get(&format!("pair{index:02}-good")).unwrap_or(&0.0);
            if good_text <= 0.0 || !holds_a_value(pair.bad) {
                continue;
            }
            let bad = *bands[band]
                .get(&format!("pair{index:02}-bad"))
                .unwrap_or(&0.0);
            alpha_min = alpha_min.max(bad / good_text);
        }
        if alpha_min >= alpha_max {
            println!("   {band}: the window is empty, so no alpha is printed");
            continue;
        }
        let alpha = (alpha_min.max(1e-12) * alpha_max).sqrt();

        let mut farm_scores: Vec<f64> = (0..FARMS.len())
            .map(|index| alpha * *label.get(&format!("farm{index:02}")).unwrap_or(&0.0))
            .collect();
        farm_scores.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut good_scores: Vec<f64> = Vec::new();
        let mut failures = Vec::new();
        for (index, pair) in PAIRS.iter().enumerate() {
            let good = alpha * *label.get(&format!("pair{index:02}-good")).unwrap_or(&0.0);
            let bad = if holds_a_value(pair.bad) {
                *bands[band]
                    .get(&format!("pair{index:02}-bad"))
                    .unwrap_or(&0.0)
            } else {
                alpha * *label.get(&format!("pair{index:02}-bad")).unwrap_or(&0.0)
            };
            good_scores.push(good);
            if good <= bad {
                failures.push(pair.name);
            }
        }
        good_scores.sort_by(|a, b| a.partial_cmp(b).unwrap());

        println!("   {band}: alpha {alpha:.6}, bar {bar:.6}");
        println!("      farm  {farm_scores:.6?}");
        println!("      label {good_scores:.6?}");
        println!(
            "      worst farm {:.6}  best farm {:.6}  bar {:.6}  {}",
            farm_scores.first().copied().unwrap_or(0.0),
            farm_scores.last().copied().unwrap_or(0.0),
            bar,
            if farm_scores.last().copied().unwrap_or(0.0) < bar {
                "every farm is under the bar"
            } else {
                "A FARM REACHES THE BAR"
            }
        );
        if failures.is_empty() {
            println!("      every label pair separates");
        } else {
            println!("      PAIRS THAT DO NOT SEPARATE: {}", failures.join(" "));
        }
        println!();
    }
    Ok(())
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let outcome = match mode.as_str() {
        "--emit" => emit(),
        "--report" => report(),
        "--attenuate" => attenuate(),
        _ => {
            println!("usage: rule6_probe --emit | --report | --attenuate");
            Ok(())
        }
    };
    if let Err(error) = outcome {
        println!("failed: {error}");
        std::process::exit(1);
    }
}
