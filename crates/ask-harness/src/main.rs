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
mod cache;
mod challenge;
mod eip712;
mod sign;

use std::time::Duration;

use ask::{fetch_challenge, prepare_payment, send_paid_ask, ALLOWED_CHAIN_ID, PAYMENT_HEADER};
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

    let question = "What is the weather in Tokyo?";
    let intent = "WEATHER_CHECK";
    println!("question: {question:?}");
    println!("intent:   {intent}");
    println!();

    println!("--- the live challenge ---");
    let challenge = fetch_challenge(ENDPOINT, question, intent)?;
    print_challenge(&challenge);
    println!();

    let leg = challenge
        .eip155_leg(ALLOWED_CHAIN_ID)
        .ok_or_else(|| format!("the node offers no eip155:{ALLOWED_CHAIN_ID} leg"))?;

    println!("--- the signed payment ---");
    let payment = prepare_payment(leg, &signer, question.as_bytes())?;
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
    println!("NOTHING WAS SENT. NOTHING WAS SPENT.");
    Ok(())
}

/// This function runs exactly ONE paid ask, then stops.
fn run_once() -> Result<(), String> {
    println!("=== ONE REAL ASK ===");
    println!();

    let (signer, _) = load_signer(false)?;
    println!("payer address: {}", to_hex(&signer.address()));

    let question = "What is the weather in Tokyo?";
    let intent = "WEATHER_CHECK";

    let challenge = fetch_challenge(ENDPOINT, question, intent)?;
    let leg = challenge
        .eip155_leg(ALLOWED_CHAIN_ID)
        .ok_or_else(|| format!("the node offers no eip155:{ALLOWED_CHAIN_ID} leg"))?;
    let payment = prepare_payment(leg, &signer, question.as_bytes())?;

    println!("spending {} smallest units on one ask", payment.value);
    let outcome = send_paid_ask(ENDPOINT, question, intent, &payment)?;

    println!("status: {}", outcome.status);
    println!("body:");
    println!("{}", outcome.body);

    cache::store("once", question, &outcome.body)?;
    println!();
    println!(
        "total spent: {} smallest units (1 ask)",
        outcome.spent_units
    );
    println!("STOPPING. Check the response, then run `probe`.");
    Ok(())
}

/// This constant records the budget default for a reader.
///
/// The batch path is not built yet. It stays behind the `probe` gate,
/// which decides whether head-to-head data is obtainable at all.
#[allow(dead_code)]
const BUDGET_NOTE: u64 = DEFAULT_BUDGET_ASKS;

/// This constant records the ask interval for a reader.
#[allow(dead_code)]
const INTERVAL_NOTE: Duration = ASK_INTERVAL;
