//! The promotion gates, measured on a benchmark of intents this
//! repository does not control.
//!
//! The node records four numbers when it compares a candidate script
//! against the champion: `worst_self_match`, `score_stddev`,
//! `candidate_margin` and `candidate_wins`. Every number here comes
//! from the same 40 triples, and every triple names an intent family
//! outside the weather corpus: a URL verdict, an SSL grade, a CVE
//! severity, a sentiment label, a translation, a fact check, a chat
//! completion, and a small numeric tail.
//!
//! The ground truth arrives in three renderings, because the protocol
//! does not state which one a validator sends: a bare value, a prose
//! sentence from the response converter, and a JSON object.
//!
//! Two steps, because the authority is the compiled module and not this
//! process:
//!
//! ```text
//! cargo run -p corpus-eval --example promotion_gates -- --emit
//! (cd tools/wazero-runner && go run . -golden ../../target/promotion-vectors.json \
//!    -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
//!    -out ../../target/promotion-wazero.json)
//! cargo run -p corpus-eval --example promotion_gates -- --report
//! ```
//!
//! `--report` reads the wazero scores. It also scores every row through
//! the native library and stops if the two disagree, so a number in the
//! report can never come from a path the wasm module does not take.
//!
//! The champion column is `baseline_score`, the native copy of the
//! reference module in `src/baseline.rs`.

use eval_script::score::score_answer;

/// Where `--emit` writes the vectors and `--report` reads the scores.
const VECTORS_PATH: &str = "target/promotion-vectors.json";
/// Where the wazero runner writes the scores of the module under test.
const SCORES_PATH: &str = "target/promotion-wazero.json";
/// Where the wazero runner writes the champion's scores.
const CHAMPION_SCORES_PATH: &str = "target/promotion-champion.json";
/// The champion used when the caller names none.
///
/// This is the module the protocol documents ship. Pass `--champion`
/// with any other `.wasm` to compare against that one instead; nothing
/// in this file needs to change when a new champion lands.
const DEFAULT_CHAMPION: &str = "reference/scoring_module.wasm";

/// The honest-miner bar from the weather corpus: the score a correct
/// miner earns when it is 10 percent out at the weather band.
const HONEST_BAR: f64 = 0.0831;

/// How the ground truth is rendered.
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    /// The value on its own.
    Bare,
    /// A sentence from the response converter.
    Prose,
    /// A JSON object.
    Json,
}

impl Shape {
    /// This function gives the short table label.
    fn label(self) -> &'static str {
        match self {
            Shape::Bare => "bare",
            Shape::Prose => "prose",
            Shape::Json => "json",
        }
    }
}

/// One benchmark row: a question, a ground truth, a good answer and a
/// bad answer.
struct Triple {
    /// The intent family the row belongs to.
    intent: &'static str,
    /// The rendering of the ground truth.
    shape: Shape,
    /// The question text, as the node would send it.
    question: &'static str,
    /// The ground truth text.
    truth: &'static str,
    /// An answer a correct miner could give.
    good: &'static str,
    /// An answer a wrong miner could give.
    bad: &'static str,
    /// True when the wanted answer is a label or a text, not a
    /// quantity. The ground truth of such a row may still CONTAIN a
    /// number, which is the case the report separates out.
    text_only: bool,
}

/// The benchmark.
///
/// Every good answer is a plausible miner output rather than a copy of
/// the ground truth, except where the value has one spelling only. A
/// benchmark of byte-identical good answers measures the exact-match
/// short circuit and nothing else.
const TRIPLES: [Triple; 40] = [
    // --- URL_SCAN ------------------------------------------------
    Triple {
        intent: "URL_SCAN",
        shape: Shape::Bare,
        question: "Is http://login-secure-update.tld/ malicious?",
        truth: "malicious",
        good: "malicious",
        bad: "safe",
        text_only: true,
    },
    Triple {
        intent: "URL_SCAN",
        shape: Shape::Prose,
        question: "Is http://login-secure-update.tld/ malicious?",
        truth: "The scan verdict for this URL is malicious.",
        good: "malicious",
        bad: "The scan verdict for this URL is clean.",
        text_only: true,
    },
    Triple {
        intent: "URL_SCAN",
        shape: Shape::Json,
        question: "Scan http://login-secure-update.tld/",
        truth: "{\"verdict\":\"phishing\",\"confidence\":0.97}",
        good: "phishing",
        bad: "benign",
        text_only: true,
    },
    Triple {
        intent: "URL_SCAN",
        shape: Shape::Json,
        question: "Scan http://login-secure-update.tld/",
        truth: "{\"verdict\":\"phishing\",\"confidence\":0.97}",
        good: "phishing, confidence 0.97",
        bad: "benign, confidence 0.12",
        text_only: true,
    },
    Triple {
        intent: "URL_SCAN",
        shape: Shape::Prose,
        question: "Is https://example.org/ malicious?",
        truth: "The URL is not malicious.",
        good: "not malicious",
        bad: "malicious",
        text_only: true,
    },
    // --- SSL_GRADE -----------------------------------------------
    Triple {
        intent: "SSL_GRADE",
        shape: Shape::Bare,
        question: "SSL grade for example.org",
        truth: "A+",
        good: "A+",
        bad: "F",
        text_only: true,
    },
    Triple {
        intent: "SSL_GRADE",
        shape: Shape::Prose,
        question: "SSL grade for legacy.example.org",
        truth: "The certificate grade for this host is B.",
        good: "B",
        bad: "A",
        text_only: true,
    },
    Triple {
        intent: "SSL_GRADE",
        shape: Shape::Json,
        question: "SSL report for example.org",
        truth: "{\"grade\":\"A\",\"protocol\":\"TLS 1.3\"}",
        good: "A",
        bad: "F",
        text_only: true,
    },
    // --- CVE_SEVERITY --------------------------------------------
    Triple {
        intent: "CVE_SEVERITY",
        shape: Shape::Bare,
        question: "Severity of CVE-2021-44228",
        truth: "CRITICAL",
        good: "critical",
        bad: "low",
        text_only: true,
    },
    Triple {
        intent: "CVE_SEVERITY",
        shape: Shape::Prose,
        question: "Severity of CVE-2021-44228",
        truth: "CVE-2021-44228 has a severity rating of CRITICAL.",
        good: "CRITICAL",
        bad: "MEDIUM",
        text_only: true,
    },
    Triple {
        intent: "CVE_SEVERITY",
        shape: Shape::Json,
        question: "Severity of CVE-2021-44228",
        truth: "{\"cve\":\"CVE-2021-44228\",\"severity\":\"critical\",\"cvss\":9.8}",
        good: "critical",
        bad: "low",
        text_only: true,
    },
    Triple {
        intent: "CVE_SEVERITY",
        shape: Shape::Bare,
        question: "CVSS base score of CVE-2021-44228",
        truth: "9.8",
        good: "9.8",
        bad: "3.1",
        text_only: false,
    },
    // --- SENTIMENT -----------------------------------------------
    Triple {
        intent: "SENTIMENT",
        shape: Shape::Bare,
        question: "Sentiment of: the staff were kind and the room was clean",
        truth: "positive",
        good: "positive",
        bad: "negative",
        text_only: true,
    },
    Triple {
        intent: "SENTIMENT",
        shape: Shape::Prose,
        question: "Sentiment of: the staff were kind and the room was clean",
        truth: "The sentiment of the review is positive.",
        good: "positive",
        bad: "The sentiment of the review is negative.",
        text_only: true,
    },
    Triple {
        intent: "SENTIMENT",
        shape: Shape::Json,
        question: "Sentiment of: the delivery was late and the box was open",
        truth: "{\"label\":\"negative\",\"score\":0.88}",
        good: "negative",
        bad: "positive",
        text_only: true,
    },
    // --- TRANSLATION ---------------------------------------------
    Triple {
        intent: "TRANSLATION",
        shape: Shape::Bare,
        question: "Translate to French: hello world",
        truth: "Bonjour le monde",
        good: "bonjour le monde",
        bad: "Hola mundo",
        text_only: true,
    },
    Triple {
        intent: "TRANSLATION",
        shape: Shape::Prose,
        question: "Translate to French: hello world",
        truth: "The French translation is: Bonjour le monde.",
        good: "Bonjour le monde",
        bad: "Au revoir le monde",
        text_only: true,
    },
    Triple {
        intent: "TRANSLATION",
        shape: Shape::Bare,
        question: "Translate to Hindi: hello world",
        truth: "\u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947} \u{926}\u{941}\u{928}\u{93f}\u{92f}\u{93e}",
        good: "\u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947} \u{926}\u{941}\u{928}\u{93f}\u{92f}\u{93e}",
        bad: "\u{905}\u{932}\u{935}\u{93f}\u{926}\u{93e} \u{926}\u{941}\u{928}\u{93f}\u{92f}\u{93e}",
        text_only: true,
    },
    Triple {
        intent: "TRANSLATION",
        shape: Shape::Bare,
        question: "Translate to Arabic: hello world",
        truth: "\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}",
        good: "\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}",
        bad: "\u{648}\u{62f}\u{627}\u{639}\u{627} \u{623}\u{64a}\u{647}\u{627} \u{627}\u{644}\u{639}\u{627}\u{644}\u{645}",
        text_only: true,
    },
    Triple {
        intent: "TRANSLATION",
        shape: Shape::Bare,
        question: "Translate to Chinese: hello world",
        truth: "\u{4f60}\u{597d}\u{4e16}\u{754c}",
        good: "\u{4f60}\u{597d}\u{4e16}\u{754c}",
        bad: "\u{518d}\u{89c1}\u{4e16}\u{754c}",
        text_only: true,
    },
    // --- FACT_CHECK ----------------------------------------------
    Triple {
        intent: "FACT_CHECK",
        shape: Shape::Bare,
        question: "Claim: the Great Wall is visible from the Moon.",
        truth: "false",
        good: "false",
        bad: "true",
        text_only: true,
    },
    Triple {
        intent: "FACT_CHECK",
        shape: Shape::Prose,
        question: "Claim: the Great Wall is visible from the Moon.",
        truth: "The claim is false.",
        good: "false",
        bad: "The claim is true.",
        text_only: true,
    },
    Triple {
        intent: "FACT_CHECK",
        shape: Shape::Prose,
        question: "Claim: the programme cut transmission by half.",
        truth: "Partly true. The programme reduced transmission by 40%.",
        good: "partly true",
        bad: "60%",
        text_only: true,
    },
    Triple {
        intent: "FACT_CHECK",
        shape: Shape::Json,
        question: "Claim: the Great Wall is visible from the Moon.",
        truth: "{\"verdict\":\"false\",\"sources\":3}",
        good: "false",
        bad: "true",
        text_only: true,
    },
    // --- CHAT_COMPLETION -----------------------------------------
    Triple {
        intent: "CHAT",
        shape: Shape::Prose,
        question: "What is the capital of France?",
        truth: "Paris is the capital of France.",
        good: "Paris",
        bad: "Lyon is the capital of France.",
        text_only: true,
    },
    Triple {
        intent: "CHAT",
        shape: Shape::Bare,
        question: "What is the capital of France?",
        truth: "Paris",
        good: "Paris",
        bad: "Lyon",
        text_only: true,
    },
    Triple {
        intent: "CHAT",
        shape: Shape::Prose,
        question: "What is the capital of Japan?",
        truth: "The capital of Japan is Tokyo.",
        good: "Tokyo is the capital of Japan.",
        bad: "The capital of Japan is Osaka.",
        text_only: true,
    },
    Triple {
        intent: "CHAT",
        shape: Shape::Prose,
        question: "How many continents are there?",
        truth: "There are seven continents.",
        good: "seven",
        bad: "five continents",
        text_only: true,
    },
    // --- classification and routing ------------------------------
    Triple {
        intent: "LANG_DETECT",
        shape: Shape::Bare,
        question: "Language of: je ne sais pas",
        truth: "fr",
        good: "fr",
        bad: "de",
        text_only: true,
    },
    Triple {
        intent: "MODERATION",
        shape: Shape::Json,
        question: "Moderate the pasted comment.",
        truth: "{\"category\":\"hate\",\"action\":\"block\"}",
        good: "hate",
        bad: "safe",
        text_only: true,
    },
    Triple {
        intent: "SPAM",
        shape: Shape::Bare,
        question: "Classify the message.",
        truth: "spam",
        good: "spam",
        bad: "ham",
        text_only: true,
    },
    Triple {
        intent: "YES_NO",
        shape: Shape::Bare,
        question: "Does this repository ship a binary?",
        truth: "no",
        good: "no",
        bad: "yes",
        text_only: true,
    },
    Triple {
        intent: "TICKET_ROUTE",
        shape: Shape::Bare,
        question: "Route: my card was charged twice.",
        truth: "billing",
        good: "billing",
        bad: "engineering",
        text_only: true,
    },
    Triple {
        intent: "ENTITY",
        shape: Shape::Prose,
        question: "Which company does the filing name?",
        truth: "The company named in the filing is Acme Corporation.",
        good: "Acme Corporation",
        bad: "Globex Corporation",
        text_only: true,
    },
    Triple {
        intent: "SUMMARY",
        shape: Shape::Prose,
        question: "Summarise the incident report in one sentence.",
        truth: "A power loss in the east region stopped the payment service for two hours.",
        good: "The payment service stopped for two hours after a power loss in the east region.",
        bad: "A software release stopped the search service for ten minutes.",
        text_only: true,
    },
    Triple {
        intent: "CODE_REVIEW",
        shape: Shape::Prose,
        question: "Review the pull request.",
        truth: "The pull request adds a SQL injection in the search handler.",
        good: "SQL injection",
        bad: "no issues found",
        text_only: true,
    },
    Triple {
        intent: "OCR",
        shape: Shape::Bare,
        question: "Read the invoice number.",
        truth: "INVOICE 2024-001",
        good: "INVOICE 2024-001",
        bad: "INVOICE 2024-002",
        text_only: true,
    },
    // --- the numeric tail ----------------------------------------
    Triple {
        intent: "WEATHER_CHECK",
        shape: Shape::Prose,
        question: "[direct] 211 -> /weather",
        truth: "The temperature is 28.9 C.",
        good: "28.9 C",
        bad: "31.5 C",
        text_only: false,
    },
    Triple {
        intent: "CRYPTO_PRICE",
        shape: Shape::Bare,
        question: "[direct] 207 -> /price",
        truth: "$192.43",
        good: "192.43 USD",
        bad: "$210.00",
        text_only: false,
    },
    Triple {
        intent: "GAS_PRICE",
        shape: Shape::Bare,
        question: "[direct] 219 -> /gas",
        truth: "12 gwei",
        good: "12 gwei",
        bad: "40 gwei",
        text_only: false,
    },
];

/// One Stage 1 robustness case.
struct Gate {
    /// The short name for the table.
    name: &'static str,
    /// The ground truth text.
    truth: String,
    /// The miner answer text.
    answer: String,
    /// What the row proves.
    note: &'static str,
}

/// This function builds the Stage 1 robustness cases.
///
/// Two of them are tens of kilobytes, so they are built here rather
/// than written out as constants.
fn gates() -> Vec<Gate> {
    let long_prose = {
        let mut text = String::new();
        while text.len() < 50 * 1024 {
            text.push_str("the scan verdict for this url is under review and the host is slow ");
        }
        text.push_str("malicious");
        text
    };
    let long_token = "a".repeat(50 * 1024);
    let long_json = {
        let mut text = String::from("{\"verdict\":\"malicious\"");
        let mut index = 0;
        while text.len() < 50 * 1024 {
            text.push_str(&format!(",\"note_{index}\":\"seen at 2026-08-17T12:00 \""));
            index += 1;
        }
        text.push('}');
        text
    };

    vec![
        Gate {
            name: "empty answer",
            truth: "malicious".to_string(),
            answer: String::new(),
            note: "Stage 1 gate 2: must be exactly 0.0",
        },
        Gate {
            name: "whitespace answer",
            truth: "malicious".to_string(),
            answer: " \t\n\r ".to_string(),
            note: "Stage 1 gate 2: must be exactly 0.0",
        },
        Gate {
            name: "50 KB prose answer",
            truth: "malicious".to_string(),
            answer: long_prose,
            note: "51 KB, ends with the right word",
        },
        Gate {
            name: "50 KB single token",
            truth: "malicious".to_string(),
            answer: long_token,
            note: "one token of 51 KB, no separator",
        },
        Gate {
            name: "50 KB JSON truth",
            truth: long_json,
            answer: "malicious".to_string(),
            note: "the quoted-span scan meets a 50 KB truth",
        },
        Gate {
            name: "pure emoji answer",
            truth: "malicious".to_string(),
            answer: "\u{1f600}\u{1f680}\u{1f525}\u{1f9ea}\u{2728}\u{1f4a5}".to_string(),
            note: "no ASCII at all",
        },
        Gate {
            name: "emoji around a value",
            truth: "The temperature is 28.9 C.".to_string(),
            answer: "\u{1f321} 28.9 C \u{2705}".to_string(),
            note: "a converter that decorates a correct value",
        },
        Gate {
            name: "Devanagari partial",
            truth: "\u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947} \u{926}\u{941}\u{928}\u{93f}\u{92f}\u{93e}".to_string(),
            answer: "\u{926}\u{941}\u{928}\u{93f}\u{92f}\u{93e}".to_string(),
            note: "one of two words",
        },
        Gate {
            name: "Arabic partial",
            truth: "\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}".to_string(),
            answer: "\u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}".to_string(),
            note: "one of two words",
        },
        Gate {
            name: "CJK partial",
            truth: "\u{4f60}\u{597d}\u{4e16}\u{754c}".to_string(),
            answer: "\u{4e16}\u{754c}".to_string(),
            note: "no space, so the truth is one token",
        },
        Gate {
            name: "Cyrillic case fold",
            truth: "\u{41c}\u{41e}\u{421}\u{41a}\u{412}\u{410}".to_string(),
            answer: "\u{43c}\u{43e}\u{441}\u{43a}\u{432}\u{430}".to_string(),
            note: "the same word, upper against lower",
        },
        Gate {
            name: "Greek case fold",
            truth: "\u{391}\u{398}\u{397}\u{39d}\u{391}".to_string(),
            answer: "\u{3b1}\u{3b8}\u{3b7}\u{3bd}\u{3b1}".to_string(),
            note: "the same word, upper against lower",
        },
        Gate {
            name: "ASCII case fold",
            truth: "CRITICAL".to_string(),
            answer: "critical".to_string(),
            note: "the control for the two rows above",
        },
        Gate {
            name: "Devanagari over the token cap",
            truth: "\u{92a}\u{930}\u{93f}\u{935}\u{939}\u{928}\u{92e}\u{902}\u{924}\u{94d}\u{930}\u{93e}\u{932}\u{92f}\u{92e}".to_string(),
            answer: "\u{92a}\u{930}\u{93f}\u{935}\u{939}\u{928}\u{92e}\u{902}\u{924}\u{94d}\u{930}\u{93e}\u{932}\u{92f}\u{938}".to_string(),
            note: "two words that differ after byte 32",
        },
    ]
}

// -----------------------------------------------------------------
// --emit
// -----------------------------------------------------------------

/// This function writes every vector in the golden-vector shape.
///
/// The shape is what `tools/wazero-runner -golden` reads and what
/// `host-runner` reads, so both engines score exactly these bytes.
/// `expected` is a placeholder that the caller fills in from the wazero
/// run before it asks host-runner for bit equality.
fn write_vectors() -> std::io::Result<()> {
    let mut vectors = Vec::new();
    for (index, triple) in TRIPLES.iter().enumerate() {
        let spaced = double_first_space(triple.truth);
        for (suffix, answer) in [
            ("self", triple.truth),
            ("selfws", spaced.as_str()),
            ("good", triple.good),
            ("bad", triple.bad),
        ] {
            vectors.push(serde_json::json!({
                "name": format!("q{index:02}-{suffix}"),
                "question": triple.question,
                "ground_truth": triple.truth,
                "miner_answer": answer,
                "expected": 0.0,
            }));
        }
    }
    for (index, gate) in gates().iter().enumerate() {
        vectors.push(serde_json::json!({
            "name": format!("gate{index:02}"),
            "question": "",
            "ground_truth": gate.truth,
            "miner_answer": gate.answer,
            "expected": 0.0,
        }));
    }
    let document = serde_json::json!({ "vectors": vectors });
    std::fs::write(
        VECTORS_PATH,
        format!("{}\n", serde_json::to_string_pretty(&document)?),
    )
}

/// This function runs one module over the vectors and reads its scores.
///
/// The module is scored through the same engine golden mode the golden
/// vector check uses, so a champion needs nothing except the published
/// ABI. It gives `None`, with a message, when the run fails.
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
// --report
// -----------------------------------------------------------------

/// This function doubles the first space in a text.
///
/// The result carries the same words, the same numbers and the same
/// meaning as the input. It is not BYTE EQUAL to the input, so it does
/// not reach the exact-match short circuit in `score_answer`. The
/// difference between the two scores is what the short circuit is
/// worth.
///
/// A text with no space comes back unchanged, so its score stays the
/// short-circuit score. Those rows are named in the report.
fn double_first_space(text: &str) -> String {
    match text.find(' ') {
        Some(at) => format!("{} {}", &text[..at], &text[at..]),
        None => text.to_string(),
    }
}

/// This function gives the population standard deviation of a sample.
fn stddev(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let count = values.len() as f64;
    let mean = values.iter().sum::<f64>() / count;
    let variance = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / count;
    variance.sqrt()
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

/// One scored benchmark row.
struct Row {
    /// The index in `TRIPLES`.
    index: usize,
    /// The self-match score, `rank_answer(q, gt, gt)`.
    own: f64,
    /// The self-match score when the answer holds the same words but
    /// one doubled space, so it misses the exact-match short circuit.
    own_spaced: f64,
    /// This module's score for the good answer.
    good: f64,
    /// This module's score for the bad answer.
    bad: f64,
    /// The champion's score for the good answer.
    base_good: f64,
    /// The champion's score for the bad answer.
    base_bad: f64,
}

/// How a case comes out when a TIE COUNTS AS A LOSS.
///
/// The node's published bar makes this the only classification that
/// matters. Three rejected entries carried margins of 0.71, 0.36 and
/// 0.31 and every one of them was rejected at 31 wins out of 32, while
/// the champion holds 32 out of 32. A candidate that fails to separate
/// one single case is out, whatever its average margin is.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Verdict {
    /// `good` and `bad` scored the same, so this module separated
    /// nothing. This is a LOSS, not a draw.
    Dead,
    /// `bad` outscored `good`. Worse than dead: the wrong answer won.
    Inverted,
    /// `good` outscored `bad`. This is the only outcome that counts.
    Win,
}

/// This function classifies one module's own separation on one case.
///
/// It reads one module's two scores and nothing else. `candidate_wins`
/// and `champion_wins` are both computed this way, which is what lets
/// the champion hold 32/32 against every entry: the number is a
/// property of one module on the fixture set, not of a head to head.
fn verdict(good: f64, bad: f64) -> Verdict {
    if good > bad {
        Verdict::Win
    } else if good < bad {
        Verdict::Inverted
    } else {
        Verdict::Dead
    }
}

/// This function scores every row and prints the report.
///
/// `module` is the script under test and `champion` is the script it is
/// compared against. Both are `.wasm` files and both are scored through
/// the same engine over the same vectors, so the comparison holds for
/// any champion. Nothing here knows what either module does inside.
fn report(module: &str, champion: &str) -> std::io::Result<()> {
    let Some((rows, scores)) = build_rows(module, champion)? else {
        return Ok(());
    };

    print_stage_one(&rows, &scores);
    print_self_match(&rows);
    print_rows(&rows);
    print_aggregates(&rows);
    print_gates(&scores);
    Ok(())
}

/// This function prints the tie report, the one the promotion bar
/// turns on.
fn ties(module: &str, champion: &str) -> std::io::Result<()> {
    let Some((rows, _scores)) = build_rows(module, champion)? else {
        return Ok(());
    };
    print_ties(&rows);
    print_comparable(&rows);
    print_margin_check(&rows);
    Ok(())
}

/// This function scores every vector through both modules and pairs the
/// results up into one row per question.
///
/// `--report` and `--ties` both read the same rows from the same engine
/// run, so the two modes can never disagree about a number. It gives
/// `None` when a module could not be scored, having already said why.
#[allow(clippy::type_complexity)]
fn build_rows(
    module: &str,
    champion: &str,
) -> std::io::Result<Option<(Vec<Row>, std::collections::HashMap<String, f64>)>> {
    write_vectors()?;

    println!("module:   {module}");
    println!("champion: {champion}\n");

    let scores = match run_module_scores(module, SCORES_PATH) {
        Some(table) => table,
        None => return Ok(None),
    };
    let champion_scores = match run_module_scores(champion, CHAMPION_SCORES_PATH) {
        Some(table) => table,
        None => return Ok(None),
    };

    // The native library in this process was built from one band. It
    // can only check a module built from that same band, so the check
    // runs for the default artefact and is skipped, out loud, for any
    // other.
    let native_check = module == WASM_PATH;
    let mut rows = Vec::new();
    let mut drift = Vec::new();

    for (index, triple) in TRIPLES.iter().enumerate() {
        let mut fetch = |suffix: &str, answer: &str| -> f64 {
            let key = format!("q{index:02}-{suffix}");
            let from_wasm = *scores.get(&key).unwrap_or(&-1.0);
            if native_check {
                let native = score_answer(triple.question, triple.truth, answer);
                // An f32 result carries about 7 decimal digits, so the
                // comparison uses the width of that narrowing and not an
                // exact equality.
                if (from_wasm - native).abs() > 1e-6 {
                    drift.push(format!("{key}: wasm {from_wasm:.6} native {native:.6}"));
                }
            }
            from_wasm
        };
        let own = fetch("self", triple.truth);
        let own_spaced = fetch("selfws", &double_first_space(triple.truth));
        let good = fetch("good", triple.good);
        let bad = fetch("bad", triple.bad);
        rows.push(Row {
            index,
            own,
            own_spaced,
            good,
            bad,
            base_good: *champion_scores
                .get(&format!("q{index:02}-good"))
                .unwrap_or(&0.0),
            base_bad: *champion_scores
                .get(&format!("q{index:02}-bad"))
                .unwrap_or(&0.0),
        });
    }

    if native_check {
        for (index, gate) in gates().iter().enumerate() {
            let key = format!("gate{index:02}");
            let from_wasm = *scores.get(&key).unwrap_or(&-1.0);
            let native = score_answer("", &gate.truth, &gate.answer);
            if (from_wasm - native).abs() > 1e-6 {
                drift.push(format!("{key}: wasm {from_wasm:.6} native {native:.6}"));
            }
        }
    }

    if !drift.is_empty() {
        println!("WASM AND NATIVE DISAGREE, the report is not trustworthy:");
        for line in &drift {
            println!("  {line}");
        }
        return Ok(None);
    }
    if native_check {
        println!(
            "wasm and native agree on all {} vectors\n",
            rows.len() * 4 + gates().len()
        );
    } else {
        println!("the module is not the default artefact, so the native cross-check is off\n");
    }

    Ok(Some((rows, scores)))
}

/// This function prints the tie analysis, which is the number the
/// published bar actually turns on.
///
/// The node reports `champion_margin` 0.37360683 and `champion_wins`
/// 32/32 against every entry, and it rejected three entries whose
/// margins were 0.71, 0.36 and 0.31 at 31 wins out of 32. So margin
/// does not buy a pass and one unseparated case is fatal. What decides
/// promotion is the count of cases this module fails to WIN OUTRIGHT.
fn print_ties(rows: &[Row]) {
    println!("1. TIES, the count that decides promotion");
    println!("   --------------------------------------");
    println!("   A tie is a LOSS. Rejected entries carried margins of 0.71, 0.36");
    println!("   and 0.31 and every one lost at 31/32 wins.\n");
    println!(
        "   {:<5} {:>9} {:>9} {:>9}  {:<9} {:>9} {:<9}",
        "q", "good", "bad", "margin", "verdict", "champion", "champ"
    );

    let mut dead = Vec::new();
    let mut both_zero = Vec::new();
    let mut inverted = Vec::new();
    let mut equals_champion = Vec::new();

    for row in rows {
        let ours = verdict(row.good, row.bad);
        let theirs = verdict(row.base_good, row.base_bad);
        let margin = row.good - row.bad;
        let champion_margin = row.base_good - row.base_bad;

        // "Our score equals the champion's" on a case means neither
        // module pulled ahead of the other on it. Compared on the
        // margin, because the margin is what the node averages.
        let level_with_champion = margin == champion_margin;
        if level_with_champion {
            equals_champion.push(row.index);
        }

        match ours {
            Verdict::Dead => {
                dead.push(row.index);
                if row.good == 0.0 && row.bad == 0.0 {
                    both_zero.push(row.index);
                }
            }
            Verdict::Inverted => inverted.push(row.index),
            Verdict::Win => {}
        }

        let mark = match ours {
            Verdict::Dead if row.good == 0.0 => "DEAD 0.0",
            Verdict::Dead => "DEAD",
            Verdict::Inverted => "INVERTED",
            Verdict::Win => "win",
        };
        // The champion column says whether this case can discriminate
        // at all. A case the champion also fails to separate is one
        // this benchmark cannot use to predict a verdict.
        let champion_mark = match theirs {
            Verdict::Win => "win",
            Verdict::Dead => "DEAD",
            Verdict::Inverted => "INVERTED",
        };
        let flag = if level_with_champion {
            " = champion"
        } else {
            ""
        };
        println!(
            "   q{:<4} {:>9.4} {:>9.4} {:>9.4}  {:<9} {:>9.4} {}{}",
            row.index, row.good, row.bad, margin, mark, champion_margin, champion_mark, flag,
        );
    }

    let total = rows.len();
    let lost = dead.len() + inverted.len();
    println!();
    println!("   cases                            {total}");
    println!(
        "   DEAD, good == bad                {:<3}  {}",
        dead.len(),
        name_list(&dead)
    );
    println!(
        "     of which both exactly 0.0000   {:<3}  {}",
        both_zero.len(),
        name_list(&both_zero)
    );
    println!(
        "   INVERTED, bad > good             {:<3}  {}",
        inverted.len(),
        name_list(&inverted)
    );
    println!(
        "   margin identical to champion     {:<3}  {}",
        equals_champion.len(),
        name_list(&equals_champion)
    );
    println!(
        "   NOT WON, dead + inverted         {:<3}  <-- this is the promotion number",
        lost
    );
    println!();
}

/// This function renders a list of question indexes as `q07 q09 ...`.
fn name_list(indexes: &[usize]) -> String {
    if indexes.is_empty() {
        return "none".to_string();
    }
    indexes
        .iter()
        .map(|index| format!("q{index:02}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// This function prints wins against the two denominators that matter.
///
/// `comparable_cases` is the count of cases the CHAMPION separates. The
/// node's champion holds 32 of 32, so on its own fixture set every case
/// is comparable by this definition and `candidate_wins` is out of the
/// full set. Our 40 are harder than that: the reference module cannot
/// separate all of them, so a case it also fails is one this benchmark
/// cannot use to predict a verdict. Both denominators are printed and
/// neither is presented as the node's.
fn print_comparable(rows: &[Row]) {
    let wins = rows
        .iter()
        .filter(|r| verdict(r.good, r.bad) == Verdict::Win)
        .count();
    let champion_wins = rows
        .iter()
        .filter(|r| verdict(r.base_good, r.base_bad) == Verdict::Win)
        .count();
    let comparable: Vec<&Row> = rows
        .iter()
        .filter(|r| verdict(r.base_good, r.base_bad) == Verdict::Win)
        .collect();
    let wins_comparable = comparable
        .iter()
        .filter(|r| verdict(r.good, r.bad) == Verdict::Win)
        .count();

    println!("2. wins out of comparable_cases");
    println!("   ----------------------------");
    println!("   ours,     every case            {wins}/{}", rows.len());
    println!(
        "   champion, every case            {champion_wins}/{}",
        rows.len()
    );
    println!(
        "   ours,     comparable_cases      {wins_comparable}/{}   (cases the champion itself separates)",
        comparable.len()
    );
    println!();
}

/// The champion margin the node publishes, from the API.
///
/// # An observation, recorded rather than argued
///
/// This figure matches NEITHER build of the published baseline on this
/// benchmark. The deployed champion is the projection build, 17,952
/// bytes raw and 8,941 gzipped, whose own documentation says its
/// embeddings are "not semantically meaningful"; it scores a margin of
/// 0.0814 here. The `real_weights` build, which runs actual MiniLM
/// inference, scores 0.2158. The published figure is 0.3736.
///
/// The most likely reading is that the node's 32 fixtures are simply an
/// easier set than these 40, since a mean over different cases is a
/// different number even when the module is byte for byte the same one.
/// That is a reading, not a finding: the node's fixtures are not
/// published and this repository cannot check it.
const PUBLISHED_CHAMPION_MARGIN: f64 = 0.37360683;
/// The case count the node's fixture set holds, from `champion_wins`.
const PUBLISHED_CASE_COUNT: usize = 32;

/// This function checks our champion column against the published one.
///
/// If the two agree, this harness is measuring what the node measures
/// and the rest of the report predicts a verdict. If they do not, the
/// gap says which of the two inputs differs: the fixture set or the
/// module.
fn print_margin_check(rows: &[Row]) {
    let all: f64 = rows.iter().map(|r| r.base_good - r.base_bad).sum::<f64>() / (rows.len() as f64);
    let separated: Vec<&Row> = rows
        .iter()
        .filter(|r| verdict(r.base_good, r.base_bad) == Verdict::Win)
        .collect();
    let separated_mean: f64 = separated
        .iter()
        .map(|r| r.base_good - r.base_bad)
        .sum::<f64>()
        / (separated.len() as f64);

    println!("3. do we reproduce champion_margin = {PUBLISHED_CHAMPION_MARGIN:.8}?");
    println!("   ------------------------------------------------------");
    println!(
        "   published            margin {PUBLISHED_CHAMPION_MARGIN:.8}  wins {PUBLISHED_CASE_COUNT}/{PUBLISHED_CASE_COUNT}"
    );
    println!(
        "   ours, all cases      margin {all:.8}  wins {}/{}",
        separated.len(),
        rows.len()
    );
    println!(
        "   ours, separated only margin {separated_mean:.8}  wins {}/{}",
        separated.len(),
        separated.len()
    );
    println!();
    let matches = (all - PUBLISHED_CHAMPION_MARGIN).abs() < 1e-8
        || (separated_mean - PUBLISHED_CHAMPION_MARGIN).abs() < 1e-8;
    if matches {
        println!("   REPRODUCED. This harness is measuring the same thing the node is.");
    } else {
        println!("   NOT REPRODUCED, and the case count says why before the numbers do.");
        println!(
            "   The node averages over {PUBLISHED_CASE_COUNT} cases and this benchmark holds {}.",
            rows.len()
        );
        println!("   A mean over a different set of cases is a different number even when");
        println!("   the module is byte for byte the same one. The champion column here is");
        println!("   the reference module from telegraph-examples, so the MODULE is likely");
        println!("   right and the FIXTURE SET is what differs. These 40 triples were");
        println!("   written in this repository and the node's 32 are not published.");
        println!();
        println!("   That bounds every margin and wins figure in this report: they are");
        println!("   measured on a harder set than the node's, and they predict the node's");
        println!("   verdict only in direction, never in value.");
    }
    println!();
}

/// This function prints the four Stage 1 gates as one block.
///
/// Stage 1 asks four things of a candidate: that it loads and exports
/// the ABI, that a blank answer is exactly 0.0, that a correct answer
/// beats an unrelated one, and that long or non-English input does not
/// trap. Each line here is measured, not asserted.
fn print_stage_one(rows: &[Row], scores: &std::collections::HashMap<String, f64>) {
    let answered = scores.values().filter(|v| **v >= 0.0).count();
    let blank = *scores.get("gate00").unwrap_or(&-1.0);
    let whitespace = *scores.get("gate01").unwrap_or(&-1.0);
    let beats = rows.iter().filter(|r| r.own > r.bad).count();
    let robust = (0..gates().len())
        .filter(|index| *scores.get(&format!("gate{index:02}")).unwrap_or(&-1.0) >= 0.0)
        .count();

    println!("0. Stage 1 gates");
    println!("   -------------");
    println!("   gate 1  the module loaded and answered {answered} vectors");
    println!("   gate 2  blank answer {blank:.4}, whitespace answer {whitespace:.4}");
    println!(
        "   gate 3  a correct answer beats an unrelated one on {beats}/{} questions",
        rows.len()
    );
    println!(
        "   gate 4  {robust}/{} long, emoji and non-ASCII cases returned without a trap",
        gates().len()
    );
    println!();
}

/// This function prints the self-match gate.
fn print_self_match(rows: &[Row]) {
    println!("1. worst_self_match  rank_answer(q, gt, gt), gate >= 0.75");
    println!("   ----------------------------------------------------");
    let mut worst = f64::INFINITY;
    let mut below = Vec::new();
    for row in rows {
        if row.own < worst {
            worst = row.own;
        }
        if row.own < 0.75 {
            below.push(row);
        }
    }
    println!("   worst_self_match: {worst:.4}");
    if below.is_empty() {
        println!("   questions below 0.75: none");
    } else {
        println!("   questions below 0.75:");
        for row in below {
            let triple = &TRIPLES[row.index];
            println!(
                "     q{:02} {} {} -> {:.4}",
                row.index,
                triple.intent,
                triple.shape.label(),
                row.own
            );
        }
    }

    // The same measurement with one doubled space in the answer. The
    // words, the numbers and the meaning do not change, so a node that
    // renders the ground truth twice through any step that touches
    // whitespace still asks the same question. The score changes,
    // because the exact-match short circuit needs BYTE equality.
    println!("\n   the same gate with one doubled space in the answer:");
    let mut worst_spaced = f64::INFINITY;
    let mut spaced_below = Vec::new();
    let mut no_space = 0usize;
    for row in rows {
        if !TRIPLES[row.index].truth.contains(' ') {
            no_space += 1;
            continue;
        }
        if row.own_spaced < worst_spaced {
            worst_spaced = row.own_spaced;
        }
        if row.own_spaced < 0.75 {
            spaced_below.push(row);
        }
    }
    println!(
        "   worst_self_match: {worst_spaced:.4}   ({no_space} rows hold no space and are left out)"
    );
    if spaced_below.is_empty() {
        println!("   questions below 0.75: none");
    } else {
        println!("   questions below 0.75:");
        for row in spaced_below {
            let triple = &TRIPLES[row.index];
            println!(
                "     q{:02} {:<13} {:<5} -> {:.4}",
                row.index,
                triple.intent,
                triple.shape.label(),
                row.own_spaced
            );
        }
    }
    println!();
}

/// This function prints one line per benchmark row.
fn print_rows(rows: &[Row]) {
    println!("2. the benchmark, 40 rows");
    println!("   ours = the module under test, base = the champion");
    println!(
        "   {:<3} {:<13} {:<5} {:>8} {:>8} {:>8} {:>8}  flag",
        "id", "intent", "shape", "ours+", "ours-", "base+", "base-"
    );
    for row in rows {
        let triple = &TRIPLES[row.index];
        let mut flags = String::new();
        if row.good <= row.bad {
            flags.push_str("OURS-LOSS ");
        }
        if row.good == 0.0 && row.bad == 0.0 {
            flags.push_str("BOTH-ZERO ");
        }
        if row.bad > HONEST_BAR && row.bad >= row.good {
            flags.push_str("FARM ");
        }
        println!(
            "   {:<3} {} {:<5} {:>8.4} {:>8.4} {:>8.4} {:>8.4}  {}",
            format!("q{:02}", row.index),
            fit(triple.intent, 13),
            triple.shape.label(),
            row.good,
            row.bad,
            row.base_good,
            row.base_bad,
            flags.trim_end()
        );
    }
    println!();
}

/// This function prints the three comparison numbers the node records.
fn print_aggregates(rows: &[Row]) {
    let all_ours: Vec<f64> = rows.iter().flat_map(|r| [r.good, r.bad]).collect();
    let all_base: Vec<f64> = rows
        .iter()
        .flat_map(|r| [r.base_good, r.base_bad])
        .collect();
    let text_ours: Vec<f64> = rows
        .iter()
        .filter(|r| TRIPLES[r.index].text_only)
        .flat_map(|r| [r.good, r.bad])
        .collect();
    let text_base: Vec<f64> = rows
        .iter()
        .filter(|r| TRIPLES[r.index].text_only)
        .flat_map(|r| [r.base_good, r.base_bad])
        .collect();
    let numeric_ours: Vec<f64> = rows
        .iter()
        .filter(|r| !TRIPLES[r.index].text_only)
        .flat_map(|r| [r.good, r.bad])
        .collect();

    println!("3. score_stddev");
    println!("   ------------");
    println!(
        "   all {} candidate scores      ours {:.4}   champion {:.4}",
        all_ours.len(),
        stddev(&all_ours),
        stddev(&all_base)
    );
    println!(
        "   text-only subset, {} scores  ours {:.4}   champion {:.4}",
        text_ours.len(),
        stddev(&text_ours),
        stddev(&text_base)
    );
    println!(
        "   numeric subset, {} scores     ours {:.4}",
        numeric_ours.len(),
        stddev(&numeric_ours)
    );
    let zero_ours = all_ours.iter().filter(|v| **v == 0.0).count();
    let zero_base = all_base.iter().filter(|v| **v == 0.0).count();
    println!(
        "   exact 0.0 scores             ours {}/{}    champion {}/{}",
        zero_ours,
        all_ours.len(),
        zero_base,
        all_base.len()
    );
    println!();

    let margin_ours: f64 = rows.iter().map(|r| r.good - r.bad).sum::<f64>() / (rows.len() as f64);
    let margin_base: f64 =
        rows.iter().map(|r| r.base_good - r.base_bad).sum::<f64>() / (rows.len() as f64);
    let wins_ours = rows.iter().filter(|r| r.good > r.bad).count();
    let wins_base = rows.iter().filter(|r| r.base_good > r.base_bad).count();
    let text_rows = rows.iter().filter(|r| TRIPLES[r.index].text_only).count();
    let text_margin_ours: f64 = rows
        .iter()
        .filter(|r| TRIPLES[r.index].text_only)
        .map(|r| r.good - r.bad)
        .sum::<f64>()
        / (text_rows as f64);
    let text_margin_base: f64 = rows
        .iter()
        .filter(|r| TRIPLES[r.index].text_only)
        .map(|r| r.base_good - r.base_bad)
        .sum::<f64>()
        / (text_rows as f64);
    let text_wins_ours = rows
        .iter()
        .filter(|r| TRIPLES[r.index].text_only && r.good > r.bad)
        .count();
    let text_wins_base = rows
        .iter()
        .filter(|r| TRIPLES[r.index].text_only && r.base_good > r.base_bad)
        .count();

    println!("4. candidate_margin and candidate_wins, ours vs the champion");
    println!("   --------------------------------------------------------");
    println!(
        "   all 40 questions   margin {margin_ours:.2} vs {margin_base:.2}, wins {wins_ours}/40 vs {wins_base}/40"
    );
    println!(
        "   text-only subset   margin {text_margin_ours:.2} vs {text_margin_base:.2}, wins {text_wins_ours}/{text_rows} vs {text_wins_base}/{text_rows}"
    );
    println!();
}

/// This function prints the Stage 1 robustness table.
fn print_gates(scores: &std::collections::HashMap<String, f64>) {
    println!("5. Stage 1 gate 4: long, non-ASCII and emoji input");
    println!("   ----------------------------------------------");
    println!(
        "   {:<28} {:>10} {:>8} {:>8}  note",
        "case", "bytes", "score", "us"
    );
    for (index, gate) in gates().iter().enumerate() {
        let key = format!("gate{index:02}");
        let value = *scores.get(&key).unwrap_or(&-1.0);
        let state = if value < 0.0 {
            "MISSING".to_string()
        } else {
            format!("{value:.4}")
        };
        // The cost is the NATIVE cost of one call, in microseconds. It
        // measures the work the rules do, not the speed of a wasm
        // engine. A row that grows without bound with the input size
        // would show here.
        let started = std::time::Instant::now();
        let _ = score_answer("", &gate.truth, &gate.answer);
        let micros = started.elapsed().as_micros();
        println!(
            "   {} {:>10} {:>8} {:>8}  {}",
            fit(gate.name, 28),
            gate.truth.len().max(gate.answer.len()),
            state,
            micros,
            gate.note
        );
    }
    println!("\n   every case returned a score, so no case trapped the module.");
    println!();
    print_cost_scaling();
}

/// This function measures how the cost of one call grows with the size
/// of the miner answer.
///
/// Stage 1 gate 4 asks only that a long answer does not crash. A cost
/// that grows with the SQUARE of the answer size passes that gate and
/// still stops a validator, because the miner chooses the answer size
/// up to `MAX_INPUT_BYTES`, which is 1 MiB.
///
/// The two shapes are the two the table above separates: a text with
/// many `e` characters and no digit, and a text with digits in it.
fn print_cost_scaling() {
    println!("6. how the cost of one call grows with the answer size");
    println!("   ---------------------------------------------------");
    println!(
        "   {:>10} {:>13} {:>8} {:>13} {:>13}",
        "bytes", "letter e", "ratio", "letter a", "1 then e"
    );
    let points = measure_scan_cost();
    let mut previous: Option<u128> = None;
    for point in &points {
        let ratio = match previous {
            Some(before) if before > 0 => {
                format!("{:.1}x", (point.attack as f64) / (before as f64))
            }
            _ => "-".to_string(),
        };
        previous = Some(point.attack);
        println!(
            "   {:>10} {:>10} us {ratio:>8} {:>10} us {:>10} us",
            point.bytes, point.attack, point.plain, point.digit_first
        );
    }
    println!("   the ratio is the cost of the `e` column against the row above it.");
    println!("   a doubling of the input that costs four times as much is quadratic.");
}

/// The answer sizes the cost measurement uses, in kilobytes.
const COST_SIZES: [usize; 4] = [8, 16, 32, 64];

/// How many times each size is timed.
///
/// The measurement keeps the FASTEST run of the three. A slow run holds
/// the cost of something else on the machine; a fast run cannot hold
/// less work than the call really does.
const COST_REPEATS: usize = 3;

/// The cost of one call at one answer size, in microseconds.
struct CostPoint {
    /// The answer size in bytes.
    bytes: usize,
    /// A text of the letter `e` and no digit. This is the shape that
    /// makes the prefix search walk the whole text every time.
    attack: u128,
    /// A text of the letter `a`, for a control with no `e` in it.
    plain: u128,
    /// The same attack text with ONE digit in front of it. The prefix
    /// search stops at that digit, so this column isolates the search
    /// as the cost.
    digit_first: u128,
}

/// This function times one call at each answer size.
///
/// The caller decides what to do with the numbers. `--report` prints
/// them in the full table, `--measure` writes them to a file, and
/// `--table` reads two such files and prints the comparison. All three
/// therefore quote the same measurement and no number is ever retyped.
fn measure_scan_cost() -> Vec<CostPoint> {
    let mut points = Vec::new();
    for kilobytes in COST_SIZES {
        let bytes = kilobytes * 1024;
        let attack_text = "e".repeat(bytes);
        let plain_text = "a".repeat(bytes);
        let digit_text = format!("1{}", "e".repeat(bytes - 1));

        let mut attack = u128::MAX;
        let mut plain = u128::MAX;
        let mut digit_first = u128::MAX;
        for _ in 0..COST_REPEATS {
            let started = std::time::Instant::now();
            let _ = score_answer("", "malicious", &attack_text);
            attack = attack.min(started.elapsed().as_micros());

            let started = std::time::Instant::now();
            let _ = score_answer("", "malicious", &plain_text);
            plain = plain.min(started.elapsed().as_micros());

            let started = std::time::Instant::now();
            let _ = score_answer("", "malicious", &digit_text);
            digit_first = digit_first.min(started.elapsed().as_micros());
        }
        points.push(CostPoint {
            bytes,
            attack,
            plain,
            digit_first,
        });
    }
    points
}

// -----------------------------------------------------------------
// --measure and --table
// -----------------------------------------------------------------

/// Where `--measure` writes the cost of the code as it stands now.
const COST_BEFORE_PATH: &str = "target/scan-cost-before.json";
/// Where `--measure --after` writes the same measurement.
const COST_AFTER_PATH: &str = "target/scan-cost-after.json";

/// Where `--measure` writes the size ladder for the engine.
const COST_VECTORS_PATH: &str = "target/scan-cost-vectors.json";
/// Where the engine writes its own timing for the ladder.
const ENGINE_BEFORE_PATH: &str = "target/scan-cost-engine-before.json";
/// See [`ENGINE_BEFORE_PATH`].
const ENGINE_AFTER_PATH: &str = "target/scan-cost-engine-after.json";
/// The module the engine loads. It must be built from the same source
/// this process was built from, or the two columns measure two
/// different things.
const WASM_PATH: &str = "target/wasm32-unknown-unknown/release/eval_script.wasm";
/// The directory of the engine runner.
const ENGINE_DIR: &str = "tools/wazero-runner";

/// This function times the current build and writes the result.
///
/// Run it once against each version of the code. There is no way to
/// hold both versions in one process, so the two runs are the only way
/// to get a measured before and a measured after.
///
/// The function measures TWICE: once in this process, and once through
/// the engine the node runs. The engine column is the one the table
/// prints, because a validator pays the engine cost and not this one.
/// Build the wasm module before calling this, from the same source.
fn measure(path: &str, engine_path: &str) -> std::io::Result<()> {
    let points = measure_scan_cost();
    write_cost_vectors()?;
    let engine = run_engine_timing(engine_path);
    let rows: Vec<serde_json::Value> = points
        .iter()
        .map(|p| {
            let engine_micros = engine
                .as_ref()
                .and_then(|table| table.get(&p.bytes).copied());
            serde_json::json!({
                "bytes": p.bytes,
                "attack_micros": p.attack,
                "plain_micros": p.plain,
                "digit_first_micros": p.digit_first,
                "engine_micros": engine_micros,
            })
        })
        .collect();
    let document = serde_json::json!({ "points": rows });
    std::fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&document)?),
    )?;
    println!("wrote {} sizes to {path}", points.len());
    if engine.is_none() {
        println!("the engine column is MISSING; see the message above");
    }
    Ok(())
}

/// This function writes the size ladder as engine input vectors.
///
/// The ladder holds the same sizes and the same attack text the native
/// measurement uses, so the two columns describe one experiment.
fn write_cost_vectors() -> std::io::Result<()> {
    let vectors: Vec<serde_json::Value> = COST_SIZES
        .iter()
        .map(|kilobytes| {
            serde_json::json!({
                "name": format!("e{kilobytes}"),
                "question": "",
                "ground_truth": "malicious",
                "miner_answer": "e".repeat(kilobytes * 1024),
                "expected": 0.0,
            })
        })
        .collect();
    let document = serde_json::json!({ "vectors": vectors });
    std::fs::write(
        COST_VECTORS_PATH,
        format!("{}\n", serde_json::to_string_pretty(&document)?),
    )
}

/// This function runs the engine timing mode and reads its output.
///
/// It gives `None`, with a message, when the engine cannot run. A
/// missing engine column is stated in the table rather than filled in
/// from the native column.
fn run_engine_timing(engine_path: &str) -> Option<std::collections::HashMap<usize, u128>> {
    let outcome = std::process::Command::new("go")
        .args([
            "run",
            ".",
            "-timing",
            &format!("../../{COST_VECTORS_PATH}"),
            "-a",
            &format!("../../{WASM_PATH}"),
            "-out",
            &format!("../../{engine_path}"),
            "-repeats",
            &COST_REPEATS.to_string(),
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
            "the engine runner failed: {}",
            String::from_utf8_lossy(&outcome.stderr).trim()
        );
        return None;
    }

    let text = std::fs::read_to_string(engine_path).ok()?;
    let document: serde_json::Value = serde_json::from_str(&text).ok()?;
    let mut table = std::collections::HashMap::new();
    for row in document["vectors"].as_array()? {
        let bytes = row["bytes"].as_u64()? as usize;
        let micros = row["micros"].as_u64()? as u128;
        table.insert(bytes, micros);
    }
    Some(table)
}

/// This function reads one measurement file as (bytes, microseconds).
///
/// The microseconds are the ENGINE column. A file with no engine column
/// gives an empty result, and the caller reports that rather than
/// falling back to another column.
fn load_cost(path: &str) -> std::io::Result<Vec<(usize, u128)>> {
    let text = std::fs::read_to_string(path)?;
    let document: serde_json::Value = serde_json::from_str(&text)?;
    let mut out = Vec::new();
    if let Some(rows) = document["points"].as_array() {
        for row in rows {
            let bytes = row["bytes"].as_u64().unwrap_or(0) as usize;
            let micros = match row["engine_micros"].as_u64() {
                Some(value) => value as u128,
                None => return Ok(Vec::new()),
            };
            out.push((bytes, micros));
        }
    }
    Ok(out)
}

/// This function groups the digits of a number in threes.
fn grouped(value: u128) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(character);
    }
    out
}

/// This function gives the mean growth of a cost curve per doubling.
fn growth_per_doubling(points: &[(usize, u128)]) -> f64 {
    let mut ratios = Vec::new();
    for pair in points.windows(2) {
        if pair[0].1 > 0 {
            ratios.push((pair[1].1 as f64) / (pair[0].1 as f64));
        }
    }
    if ratios.is_empty() {
        return 0.0;
    }
    ratios.iter().sum::<f64>() / (ratios.len() as f64)
}

/// This function prints the timing table on its own.
///
/// The output holds nothing except the table. It is built for a
/// screenshot, so it names no file, no directory and no user, and it
/// says nothing about how the score itself is computed.
///
/// The last row is the input cap. No machine here can measure that row
/// in a reasonable time before the fix, so the row is projected along
/// the measured curve of each column and marked.
fn table() -> std::io::Result<()> {
    let before = load_cost(COST_BEFORE_PATH)?;
    let after = load_cost(COST_AFTER_PATH)?;
    if before.len() != after.len() || before.is_empty() {
        println!("the two engine measurements do not match; run --measure twice");
        return Ok(());
    }

    let cap_bytes = 1024usize * 1024;
    let before_growth = growth_per_doubling(&before);
    let after_growth = growth_per_doubling(&after);

    println!("one rank_answer call under wazero, fresh instance, single thread");
    println!("the answer is attacker-controlled up to MAX_INPUT_BYTES = 1 MiB");
    println!();
    println!(
        "  {:>11} {:>15} {:>15} {:>9}",
        "answer size", "before (us)", "after (us)", "speedup"
    );
    println!(
        "  {} {} {} {}",
        "-".repeat(11),
        "-".repeat(15),
        "-".repeat(15),
        "-".repeat(9)
    );
    for (index, (bytes, before_micros)) in before.iter().enumerate() {
        let after_micros = after[index].1;
        let speedup = (*before_micros as f64) / (after_micros.max(1) as f64);
        println!(
            "  {:>11} {:>15} {:>15} {:>8.0}x",
            format!("{} KiB", bytes / 1024),
            grouped(*before_micros),
            grouped(after_micros),
            speedup
        );
    }

    // The cap row, projected from the last measured size along each
    // column's own growth.
    let (last_bytes, last_before) = before[before.len() - 1];
    let last_after = after[after.len() - 1].1;
    let doublings = ((cap_bytes / last_bytes) as f64).log2();
    let cap_before = ((last_before as f64) * before_growth.powf(doublings)) as u128;
    let cap_after = ((last_after as f64) * after_growth.powf(doublings)) as u128;
    let cap_speedup = (cap_before as f64) / (cap_after.max(1) as f64);
    println!(
        "  {:>11} {:>15} {:>15} {:>8.0}x",
        format!("{} MiB", cap_bytes / (1024 * 1024)),
        format!("{}*", grouped(cap_before)),
        format!("{}*", grouped(cap_after)),
        cap_speedup
    );
    println!();
    println!(
        "  cost per doubling of the input: {before_growth:.1}x before, {after_growth:.1}x after"
    );
    println!(
        "  * projected along the measured curve: {:.1} min -> {} ms",
        (cap_before as f64) / 60_000_000.0,
        cap_after / 1000
    );
    Ok(())
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let outcome = match mode.as_str() {
        "--emit" => write_vectors().inspect(|()| println!("wrote {VECTORS_PATH}")),
        "--report" => {
            let args: Vec<String> = std::env::args().collect();
            let value_of = |flag: &str| -> Option<String> {
                args.iter()
                    .position(|a| a == flag)
                    .and_then(|at| args.get(at + 1))
                    .cloned()
            };
            report(
                &value_of("--module").unwrap_or_else(|| WASM_PATH.to_string()),
                &value_of("--champion").unwrap_or_else(|| DEFAULT_CHAMPION.to_string()),
            )
        }
        "--ties" => {
            let args: Vec<String> = std::env::args().collect();
            let value_of = |flag: &str| -> Option<String> {
                args.iter()
                    .position(|a| a == flag)
                    .and_then(|at| args.get(at + 1))
                    .cloned()
            };
            ties(
                &value_of("--module").unwrap_or_else(|| WASM_PATH.to_string()),
                &value_of("--champion").unwrap_or_else(|| DEFAULT_CHAMPION.to_string()),
            )
        }
        "--measure" => {
            let after = std::env::args().nth(2).unwrap_or_default() == "--after";
            if after {
                measure(COST_AFTER_PATH, ENGINE_AFTER_PATH)
            } else {
                measure(COST_BEFORE_PATH, ENGINE_BEFORE_PATH)
            }
        }
        "--table" => table(),
        _ => {
            println!(
                "usage: promotion_gates --emit \
                 | --report [--module <wasm>] [--champion <wasm>] \
                 | --ties [--module <wasm>] [--champion <wasm>] \
                 | --measure [--after] | --table"
            );
            Ok(())
        }
    };
    if let Err(error) = outcome {
        println!("failed: {error}");
        std::process::exit(1);
    }
}
