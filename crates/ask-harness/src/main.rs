//! `ask-harness`: buy Telegraph inference with x402 and record which
//! miner answered each ask.
//!
//! ## Why this tool exists
//!
//! Wave 4 could not rank miners. The daemon feed records ONE miner per
//! question, so there is almost no head-to-head data: 5 paraphrase
//! clusters held more than one miner, and only 2 held all three. A
//! flip rate on 2 items says nothing.
//!
//! Asking the SAME question many times and letting the protocol route
//! it gives identical input, one timestamp, and different miners. That
//! is real head-to-head data, and the daemon feed cannot supply it.
//!
//! ## Run order, which is not optional
//!
//! ```text
//! cargo run -p ask-harness -- dry-run      # signs, sends nothing
//! cargo run -p ask-harness -- once         # ONE real ask, then stop
//! cargo run -p ask-harness -- probe        # 30 asks, routing check
//! cargo run -p ask-harness -- batch        # the head-to-head set
//! ```
//!
//! Each step gates the next. `probe` is a hard gate: if every ask
//! routes to one miner, head-to-head data is not obtainable this way
//! and the work stops there.
//!
//! ## Money
//!
//! Testnet only. The signer refuses any chain other than Base Sepolia.
//! One ask costs 0.01 testnet USDC, which is free from a faucet. The
//! budget cap is a hard refusal, not a warning.

mod ask;
mod batch;
mod cache;
mod challenge;
mod eip712;
mod probe;
mod sign;

use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

use ask::{
    fetch_challenge, prepare_payment, send_paid_ask, AskOutcome, Settlement, ALLOWED_CHAIN_ID,
    PAYMENT_HEADER, SETTLEMENT_HEADER,
};
use eip712::to_hex;
use sign::Signer;

/// The dev node ask endpoint.
const ENDPOINT: &str = "https://devnode.telegraphprotocol.com/engine/v1/ask";

/// The environment variable that holds the signing key.
const KEY_VARIABLE: &str = "ASK_HARNESS_PRIVATE_KEY";

/// The default budget, in asks.
const DEFAULT_BUDGET_ASKS: u64 = 50;

/// The wait between asks, so the dev node is not hammered.
const ASK_INTERVAL: Duration = Duration::from_secs(2);

/// A well known TEST key, used by `dry-run` only when no real key is
/// set.
///
/// It holds nothing and it must never be funded. A dry run sends
/// nothing, so a demonstration key is safe there and nowhere else.
const DEMO_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let command = arguments.get(1).map(String::as_str).unwrap_or("help");

    let outcome = match command {
        "dry-run" => run_dry(),
        "once" => run_once(),
        "probe" => run_probe(&arguments),
        "batch" => run_batch(&arguments),
        _ => {
            eprintln!("usage: ask-harness <dry-run|once|probe|batch> [--budget N]");
            eprintln!();
            eprintln!("run them in that order. each one gates the next.");
            std::process::exit(2);
        }
    };

    if let Err(message) = outcome {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

/// This function reads the signing key from the environment.
///
/// The function never prints the key. It prints the ADDRESS, which is
/// public and which the caller needs in order to fund the wallet.
fn load_signer(allow_demo: bool) -> Result<(Signer, bool), String> {
    match std::env::var(KEY_VARIABLE) {
        Ok(text) if !text.trim().is_empty() => Ok((Signer::from_hex(&text)?, false)),
        _ => {
            if !allow_demo {
                return Err(format!(
                    "{KEY_VARIABLE} is not set. Export a Base Sepolia key that holds \
                     testnet USDC. The harness never writes the key anywhere."
                ));
            }
            Ok((Signer::from_hex(DEMO_KEY)?, true))
        }
    }
}

/// This function prints the decoded challenge.
fn print_challenge(challenge: &challenge::Challenge) {
    println!("x402 version: {}", challenge.x402_version);
    println!("node message: {}", challenge.error);
    println!("legs offered: {}", challenge.accepts.len());
    for leg in &challenge.accepts {
        println!("  network {}", leg.network);
        println!("    scheme  {}", leg.scheme);
        println!("    asset   {}", leg.asset);
        println!("    amount  {} (smallest unit)", leg.amount);
        println!("    payTo   {}", leg.pay_to);
        println!("    timeout {}s", leg.max_timeout_seconds);
        if let Some(name) = &leg.extra.name {
            println!("    domain name    {name}");
        }
        if let Some(version) = &leg.extra.version {
            println!("    domain version {version}");
        }
        if let Some(payer) = &leg.extra.fee_payer {
            println!("    feePayer {payer}");
        }
    }
}

/// This function runs the dry run. It sends no payment.
fn run_dry() -> Result<(), String> {
    println!("=== DRY RUN: nothing is sent, nothing is spent ===");
    println!();

    let (signer, is_demo) = load_signer(true)?;
    if is_demo {
        println!(
            "NOTE: {KEY_VARIABLE} is not set, so this dry run uses a well known\n\
             DEMO key. It holds nothing. A real ask refuses to run without a\n\
             real key."
        );
        println!();
    }
    println!("payer address: {}", to_hex(&signer.address()));
    println!();

    let query = "What is the weather in Tokyo?";
    println!("query: {query:?}");
    println!("  the Engine's router classifies the intent; the client");
    println!("  does not name a miner and does not name an intent.");
    println!();

    println!("--- the live challenge ---");
    let challenge = fetch_challenge(ENDPOINT, query)?;
    print_challenge(&challenge);
    println!();

    let leg_index = challenge
        .eip155_leg_index(ALLOWED_CHAIN_ID)
        .ok_or_else(|| format!("the node offers no eip155:{ALLOWED_CHAIN_ID} leg"))?;

    println!("--- the signed payment ---");
    let payment = prepare_payment(&challenge, leg_index, &signer, query.as_bytes())?;
    println!("domain separator: {}", to_hex(&payment.domain_separator));
    println!("  this value is checked against the deployed contract by");
    println!("  the_base_sepolia_usdc_domain_matches_the_deployed_contract");
    println!("struct hash:      {}", to_hex(&payment.struct_hash));
    println!("signing digest:   {}", to_hex(&payment.digest));
    println!("signature:        {}", to_hex(&payment.signature));
    println!("  v = {}", payment.signature[64]);
    println!("payer:            {}", to_hex(&payment.from));
    println!("value:            {} smallest units", payment.value);
    println!();
    println!("{PAYMENT_HEADER} would carry:");
    println!("  {}", payment.header_value);
    println!();
    println!("which decodes to:");
    println!("{}", decoded_envelope(&payment.header_value));
    println!();
    println!("NOTHING WAS SENT. NOTHING WAS SPENT.");
    Ok(())
}

/// This function decodes a payment header back to readable JSON.
///
/// A dry run that prints only base64 hides exactly the thing that was
/// wrong before: the envelope shape.
fn decoded_envelope(header_value: &str) -> String {
    let Some(bytes) = challenge::base64_decode(header_value) else {
        return "  (the header is not valid base64)".to_string();
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return "  (the payload is not valid UTF-8)".to_string();
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or(text),
        Err(_) => text,
    }
}

/// This function prints the settlement result of a paid ask.
///
/// The function prints on BOTH paths. A failed settlement states its
/// reason here, and that reason is the whole point of reading the
/// header.
fn print_settlement(outcome: &AskOutcome) {
    println!();
    println!("--- {SETTLEMENT_HEADER} ---");
    match &outcome.settlement {
        None => {
            println!("the node sent no {SETTLEMENT_HEADER} header.");
            println!("so it did not reach settlement; the reason is in the body above.");
        }
        Some(Err(error)) => {
            println!("the header was present but could not be read: {error}");
        }
        Some(Ok(settlement)) => {
            print_settlement_fields(settlement);
        }
    }
}

/// This function prints the fields of a decoded settlement.
fn print_settlement_fields(settlement: &Settlement) {
    match settlement.success {
        Some(true) => println!("success:     yes, the node settled the payment"),
        Some(false) => println!("success:     NO, the node refused the payment"),
        None => println!("success:     not stated"),
    }
    if let Some(reason) = &settlement.error_reason {
        println!("reason:      {reason}");
    }
    if let Some(transaction) = &settlement.transaction {
        println!("transaction: {transaction}");
    }
    if let Some(network) = &settlement.network {
        println!("network:     {network}");
    }
    if let Some(payer) = &settlement.payer {
        println!("payer:       {payer}");
    }
    println!("raw:");
    match serde_json::to_string_pretty(&settlement.raw) {
        Ok(text) => println!("{text}"),
        Err(_) => println!("{}", settlement.raw),
    }
}

/// This function runs exactly ONE paid ask, then stops.
fn run_once() -> Result<(), String> {
    println!("=== ONE REAL ASK ===");
    println!();

    let (signer, _) = load_signer(false)?;
    println!("payer address: {}", to_hex(&signer.address()));

    let query = "What is the weather in Tokyo?";

    let challenge = fetch_challenge(ENDPOINT, query)?;
    let leg_index = challenge
        .eip155_leg_index(ALLOWED_CHAIN_ID)
        .ok_or_else(|| format!("the node offers no eip155:{ALLOWED_CHAIN_ID} leg"))?;
    let payment = prepare_payment(&challenge, leg_index, &signer, query.as_bytes())?;

    println!("authorising {} smallest units for one ask", payment.value);
    println!();
    println!("--- request headers sent ---");
    println!("POST {ENDPOINT}");
    println!("Content-Type: application/json");
    println!(
        "{PAYMENT_HEADER}: {} ({} base64 characters)",
        elide(&payment.header_value),
        payment.header_value.len()
    );
    println!();
    println!("the payload decodes to:");
    println!("{}", decoded_envelope(&payment.header_value));

    let outcome = send_paid_ask(ENDPOINT, query, &payment)?;

    println!();
    println!("--- response ---");
    println!("status: {}", outcome.status);
    println!("body:");
    println!("{}", outcome.body);

    print_settlement(&outcome);

    cache::store("once", query, &outcome.body)?;

    println!();
    let settled = matches!(
        &outcome.settlement,
        Some(Ok(settlement)) if settlement.success == Some(true)
    );
    if settled {
        println!(
            "SPENT: {} smallest units on 1 ask",
            outcome.authorized_units
        );
        println!("STOPPING. Check the answer, then run `probe`.");
    } else {
        println!(
            "SPENT: 0. {} smallest units were authorised but not settled.",
            outcome.authorized_units
        );
        println!("STOPPING. Fix the reason above before sending another ask.");
    }
    Ok(())
}

/// This function shortens a long header value for the console.
///
/// The full value is printed in decoded form directly below it, so the
/// elided form is enough to confirm what was sent.
fn elide(value: &str) -> String {
    const HEAD: usize = 24;
    if value.len() <= HEAD * 2 {
        return value.to_string();
    }
    format!("{}...{}", &value[..HEAD], &value[value.len() - 8..])
}

/// How many asks the routing probe sends.
///
/// Enough to see a distribution, and not more. The point is to learn
/// whether a fixed query reaches more than one miner, which a few tens
/// of asks answers. Asking beyond that would buy no extra information
/// and would put traffic on the network for no research reason.
const PROBE_ASKS: usize = 30;

/// This function reads the `--budget N` argument.
///
/// The budget is a HARD cap in asks, checked before the first one. It
/// refuses rather than warns.
fn read_budget(arguments: &[String]) -> Result<u64, String> {
    let mut budget = DEFAULT_BUDGET_ASKS;
    let mut index = 2;
    while index < arguments.len() {
        if arguments[index] == "--budget" {
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| "--budget needs a number".to_string())?;
            budget = value
                .parse()
                .map_err(|_| format!("the budget {value:?} is not a number"))?;
            index += 2;
        } else if arguments[index] == "--plan" {
            index += 1;
        } else {
            return Err(format!("unknown argument {:?}", arguments[index]));
        }
    }
    Ok(budget)
}

/// This function runs the routing distribution probe.
///
/// The probe asks ONE fixed question many times and records which
/// miner answered each. See `probe.rs` for why this is a gate rather
/// than a result.
fn run_probe(arguments: &[String]) -> Result<(), String> {
    let budget = read_budget(arguments)?;
    if (PROBE_ASKS as u64) > budget {
        return Err(format!(
            "the probe needs {PROBE_ASKS} asks but the budget is {budget}. \
             Raise it with --budget {PROBE_ASKS} if that is what you want."
        ));
    }

    println!("=== ROUTING PROBE: {PROBE_ASKS} REAL ASKS ===");
    println!();

    let (signer, _) = load_signer(false)?;
    println!("payer address: {}", to_hex(&signer.address()));

    // A weather query, so any head-to-head rows this produces score
    // against the same archive ground truth the corpus already uses.
    let query = "What is the current weather in Tokyo?";
    println!("query:         {query:?}");
    println!("budget:        {budget} asks");
    println!(
        "cost estimate: {PROBE_ASKS} asks at 0.01 USDC = {:.2} USDC",
        (PROBE_ASKS as f64) * 0.01
    );
    println!(
        "rate:          one ask every {} seconds",
        ASK_INTERVAL.as_secs()
    );
    println!();
    println!(
        "{:>3} {:>6} {:>10} {:<22} {:<18} {:>8}",
        "#", "status", "miner_id", "miner", "intent", "settled"
    );

    let mut records = Vec::with_capacity(PROBE_ASKS);
    for index in 1..=PROBE_ASKS {
        if index > 1 {
            std::thread::sleep(ASK_INTERVAL);
        }

        let record = match ask_once(&signer, query, index) {
            Ok(outcome) => probe::AskRecord::from_outcome(index, &outcome),
            Err(message) => {
                // A transport or signing failure is recorded and the
                // run continues. Stopping would lose the asks already
                // paid for.
                println!("{index:>3} {:>6} {message}", "-");
                probe::AskRecord {
                    index,
                    status: 0,
                    settled: false,
                    transaction: None,
                    miner_id: None,
                    miner_name: None,
                    intent: None,
                    cost_usd: None,
                    signal_hash: None,
                    authorized_units: 0,
                    failure: Some(message),
                    body: String::new(),
                }
            }
        };

        if record.failure.is_none() {
            println!(
                "{:>3} {:>6} {:>10} {:<22} {:<18} {:>8}",
                record.index,
                record.status,
                record.miner_id.as_deref().unwrap_or("-"),
                record.miner_name.as_deref().unwrap_or("-"),
                record.intent.as_deref().unwrap_or("-"),
                if record.settled { "yes" } else { "NO" }
            );
        } else if record.status != 0 {
            println!(
                "{:>3} {:>6} {}",
                record.index,
                record.status,
                record.failure.as_deref().unwrap_or("failed")
            );
        }

        records.push(record);
    }

    let report = probe::summarise(&records);
    let path = probe::write_records(&records, query)?;
    let gate_open = probe::print_report(&report, PROBE_ASKS, &path);

    if !gate_open {
        // A closed gate is a real finding, not an error. It is reported
        // and the process still exits 0, because nothing went wrong.
        println!();
        println!("Not running `batch`. The remaining budget stays unspent.");
    }
    Ok(())
}

/// Where the batch writes its plan, before it spends anything.
///
/// The ground-truth step geocodes from THIS file, so the coordinates
/// come from the fixed list and never from a miner response.
const BATCH_PLAN_PATH: &str = "corpus/batch-plan.json";

/// Where the batch writes one line per ask.
const BATCH_PATH: &str = "corpus/ask-batch.jsonl";

/// This function writes the batch plan to disk.
fn write_plan() -> Result<(), String> {
    let cities: Vec<serde_json::Value> = batch::CITIES
        .iter()
        .map(|city| {
            serde_json::json!({
                "key": city.key,
                "name": city.name,
                "country": city.country,
                "utc_offset_hours": city.utc_offset_hours,
                "query": city.query(),
            })
        })
        .collect();
    let plan = serde_json::json!({
        "asks_per_city": batch::ASKS_PER_CITY,
        "total_asks": batch::total_asks(),
        "cities": cities,
    });
    if let Some(parent) = std::path::Path::new(BATCH_PLAN_PATH).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot make the corpus directory: {error}"))?;
    }
    std::fs::write(
        BATCH_PLAN_PATH,
        serde_json::to_string_pretty(&plan).unwrap_or_default(),
    )
    .map_err(|error| format!("cannot write the plan: {error}"))?;
    println!("plan written: {BATCH_PLAN_PATH}");
    Ok(())
}

/// This function runs the head-to-head batch.
///
/// `--plan` writes the plan and spends nothing, so the ground-truth
/// geocoding can be checked before any money moves.
fn run_batch(arguments: &[String]) -> Result<(), String> {
    let plan_only = arguments.iter().any(|argument| argument == "--plan");
    let budget = read_budget(arguments)?;
    let total = batch::total_asks();

    println!("=== HEAD-TO-HEAD BATCH ===");
    println!();
    println!("cities:        {}", batch::CITIES.len());
    println!("asks per city: {}", batch::ASKS_PER_CITY);
    println!("total asks:    {total}");
    println!("cost estimate: {:.2} USDC", (total as f64) * 0.01);
    println!("budget:        {budget} asks");
    println!(
        "a city misses a 23% miner in {:.4}% of runs",
        100.0 * batch::miss_probability(7.0 / 30.0, batch::ASKS_PER_CITY)
    );
    println!();

    write_plan()?;

    if plan_only {
        println!();
        println!("--plan given. NOTHING WAS SENT. NOTHING WAS SPENT.");
        return Ok(());
    }

    if (total as u64) > budget {
        return Err(format!(
            "the batch needs {total} asks but the budget is {budget}. \
             Raise it with --budget {total} if that is what you want."
        ));
    }

    let (signer, _) = load_signer(false)?;
    println!("payer address: {}", to_hex(&signer.address()));
    println!();

    let path = std::path::Path::new(BATCH_PATH);
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("cannot make the batch file: {error}"))?;

    let mut index = 0usize;
    let mut settled_units = 0u64;
    let mut failures = 0usize;

    for city in &batch::CITIES {
        let query = city.query();
        println!("--- {} ({}) ---", city.name, city.key);
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();

        for _ in 0..batch::ASKS_PER_CITY {
            index += 1;
            if index > 1 {
                std::thread::sleep(ASK_INTERVAL);
            }

            // The ask time is stamped HERE, on the client, before the
            // response exists. The ground-truth join uses this, never a
            // timestamp out of a miner response.
            let asked_at = unix_seconds()?;

            let record = match ask_once(&signer, &query, index) {
                Ok(outcome) => {
                    if matches!(&outcome.settlement, Some(Ok(s)) if s.success == Some(true)) {
                        settled_units = settled_units.saturating_add(outcome.authorized_units);
                    }
                    probe::AskRecord::from_outcome(index, &outcome)
                }
                Err(message) => {
                    println!("  ask {index}: {message}");
                    probe::AskRecord {
                        index,
                        status: 0,
                        settled: false,
                        transaction: None,
                        miner_id: None,
                        miner_name: None,
                        intent: None,
                        cost_usd: None,
                        signal_hash: None,
                        authorized_units: 0,
                        failure: Some(message),
                        body: String::new(),
                    }
                }
            };

            if record.failure.is_some() {
                failures += 1;
            } else if let Some(id) = &record.miner_id {
                *seen.entry(id.clone()).or_insert(0) += 1;
            }

            // The raw body is kept, because the ground-truth step runs
            // the corpus normaliser over it and a summary would lose
            // the fields that needs.
            let body = record_body(&record, &query, city, asked_at);
            writeln!(file, "{body}")
                .map_err(|error| format!("cannot write the batch file: {error}"))?;
        }

        let names: Vec<String> = seen
            .iter()
            .map(|(id, count)| format!("{id}x{count}"))
            .collect();
        println!(
            "  {} distinct miners: {}",
            seen.len(),
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(" ")
            }
        );
    }

    println!();
    println!("asks sent:     {index}");
    println!("failed:        {failures}");
    println!(
        "settled spend: {settled_units} smallest units ({:.2} USDC)",
        (settled_units as f64) / 1_000_000.0
    );
    println!("written:       {BATCH_PATH}");
    println!();
    println!("Now join ground truth:");
    println!("  cargo run -p corpus-eval --release -- headtohead");
    Ok(())
}

/// This function gives the current time in Unix seconds.
fn unix_seconds() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "the system clock is before the epoch".to_string())
        .map(|value| value.as_secs())
}

/// This function renders one batch line.
fn record_body(
    record: &probe::AskRecord,
    query: &str,
    city: &batch::City,
    asked_at: u64,
) -> String {
    serde_json::json!({
        "index": record.index,
        "city_key": city.key,
        "city_name": city.name,
        "city_country": city.country,
        "query": query,
        "asked_at_unix": asked_at,
        "status": record.status,
        "settled": record.settled,
        "transaction": record.transaction,
        "miner_id": record.miner_id,
        "miner_name": record.miner_name,
        "intent": record.intent,
        "cost_usd": record.cost_usd,
        "signal_hash": record.signal_hash,
        "failure": record.failure,
        "body": record.body,
    })
    .to_string()
}

/// This function runs one paid ask end to end.
///
/// The nonce seed carries the ask index, so two asks inside the same
/// second cannot build the same authorisation nonce. A repeated nonce
/// is a replay to the token contract and the second ask would be
/// refused.
fn ask_once(signer: &Signer, query: &str, index: usize) -> Result<AskOutcome, String> {
    let challenge = fetch_challenge(ENDPOINT, query)?;
    let leg_index = challenge
        .eip155_leg_index(ALLOWED_CHAIN_ID)
        .ok_or_else(|| format!("the node offers no eip155:{ALLOWED_CHAIN_ID} leg"))?;

    let mut seed = Vec::with_capacity(query.len() + 8);
    seed.extend_from_slice(query.as_bytes());
    seed.extend_from_slice(&(index as u64).to_be_bytes());

    let payment = prepare_payment(&challenge, leg_index, signer, &seed)?;
    send_paid_ask(ENDPOINT, query, &payment)
}
