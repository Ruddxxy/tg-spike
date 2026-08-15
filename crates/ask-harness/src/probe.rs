//! This module runs the routing distribution probe.
//!
//! ## What the probe decides
//!
//! Wave 4 could not rank miners, because the daemon feed routes one
//! miner per question and only 2 paraphrase clusters held all three
//! miners. The plan to fix that is to ask the SAME question many times
//! and let the Engine's router spread it over miners.
//!
//! That plan rests on an assumption nobody has tested: that routing is
//! not deterministic for a fixed query. If the router sends every ask
//! for one question to one miner, then no amount of asking produces a
//! head-to-head pair, and the rest of the budget would buy nothing.
//!
//! **This probe is a GATE, not a result.** It answers one question:
//! does a fixed query reach more than one miner? A single-miner result
//! stops the wave. It is a real finding either way, and a cheap one.
//!
//! ## Why the query is fixed and why it is a weather query
//!
//! Fixed, because varying the question would measure the router's
//! classifier rather than its miner selection. Weather, because the
//! corpus already covers the WEATHER_CHECK intent, so any head-to-head
//! rows this produces can be scored against the same archive ground
//! truth the rest of the evaluation uses.
//!
//! ## Rules this module obeys
//!
//! - The auto-routed endpoint only. Naming a miner would decide the
//!   very thing being measured.
//! - One ask every 2 seconds. A dev node is not a load target.
//! - A hard budget cap, checked BEFORE the first ask.
//! - Every ask is logged with its cost, so the total is auditable.
//! - The probe asks no more than the analysis needs. 30 asks is enough
//!   to see a distribution and is not enough to move any leaderboard.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::ask::AskOutcome;

/// Where the probe writes one line per ask.
pub const PROBE_PATH: &str = "corpus/ask-probe.jsonl";

/// One recorded ask.
#[derive(Debug, Clone)]
pub struct AskRecord {
    /// The position in the run, from 1.
    pub index: usize,
    /// The HTTP status the node returned.
    pub status: u16,
    /// Whether the node settled the payment.
    pub settled: bool,
    /// The settlement transaction, when there is one.
    pub transaction: Option<String>,
    /// The miner id the Engine routed to.
    pub miner_id: Option<String>,
    /// The miner name the Engine routed to.
    pub miner_name: Option<String>,
    /// The intent the router classified the query into.
    pub intent: Option<String>,
    /// The cost the node reported, in USD.
    pub cost_usd: Option<f64>,
    /// The node's own signal hash for this answer.
    pub signal_hash: Option<String>,
    /// The amount authorised, in the token's smallest unit.
    pub authorized_units: u64,
    /// A failure note, when the ask did not give an answer.
    pub failure: Option<String>,
    /// The raw response body.
    ///
    /// The head-to-head step runs the corpus normaliser over this, so a
    /// summary would lose exactly the fields it needs.
    pub body: String,
}

impl AskRecord {
    /// This function reads one outcome into a record.
    ///
    /// The function never fails. A body it cannot read becomes a
    /// recorded failure, because a probe that stops on the first odd
    /// answer measures less than one that records it.
    pub fn from_outcome(index: usize, outcome: &AskOutcome) -> Self {
        let settled = matches!(
            &outcome.settlement,
            Some(Ok(settlement)) if settlement.success == Some(true)
        );
        let transaction = match &outcome.settlement {
            Some(Ok(settlement)) => settlement.transaction.clone(),
            _ => None,
        };

        let parsed: Option<serde_json::Value> = serde_json::from_str(&outcome.body).ok();
        let field = |name: &str| -> Option<String> {
            parsed
                .as_ref()?
                .get(name)?
                .as_str()
                .map(std::string::ToString::to_string)
        };

        let failure = if outcome.status != 200 {
            Some(format!("http {}", outcome.status))
        } else if parsed.is_none() {
            Some("the body is not JSON".to_string())
        } else if parsed.as_ref().and_then(|v| v.get("miner_id")).is_none() {
            // A 200 with no miner names nothing to count, so it must
            // not silently become a zero row in the distribution.
            Some("the answer names no miner".to_string())
        } else {
            None
        };

        AskRecord {
            index,
            status: outcome.status,
            settled,
            transaction,
            miner_id: field("miner_id"),
            miner_name: field("miner_name"),
            intent: field("intent"),
            cost_usd: parsed
                .as_ref()
                .and_then(|v| v.get("cost_usd"))
                .and_then(serde_json::Value::as_f64),
            signal_hash: field("signal_hash"),
            authorized_units: outcome.authorized_units,
            failure,
            body: outcome.body.clone(),
        }
    }

    /// This function gives the label a distribution counts by.
    ///
    /// The id is the identity; the name is for a reader. A miner that
    /// is renamed must still count as one miner.
    pub fn miner_key(&self) -> Option<String> {
        let id = self.miner_id.as_ref()?;
        match &self.miner_name {
            Some(name) => Some(format!("{name}\t{id}")),
            None => Some(format!("(unnamed)\t{id}")),
        }
    }
}

/// The reduced result of a probe run.
pub struct ProbeReport {
    /// How many miners served at least one ask.
    pub distinct_miners: usize,
    /// The count per miner, keyed by name and id.
    pub counts: BTreeMap<String, usize>,
    /// Every ask that did not give an answer.
    pub failures: Vec<AskRecord>,
    /// How many asks the node settled.
    pub settled_asks: usize,
    /// The settled spend, in the token's smallest unit.
    pub settled_units: u64,
    /// The intents the router chose, with counts.
    pub intents: BTreeMap<String, usize>,
}

/// This function reduces the records into a report.
pub fn summarise(records: &[AskRecord]) -> ProbeReport {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut intents: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut settled_asks = 0usize;
    let mut settled_units = 0u64;

    for record in records {
        if record.settled {
            settled_asks += 1;
            settled_units = settled_units.saturating_add(record.authorized_units);
        }
        if record.failure.is_some() {
            failures.push(record.clone());
            // A failed ask names no miner, so it is not counted into
            // the distribution. It is still counted into the spend
            // above when the node settled it, because that money is
            // gone whether or not an answer came back.
            continue;
        }
        if let Some(key) = record.miner_key() {
            *counts.entry(key).or_insert(0) += 1;
        }
        if let Some(intent) = &record.intent {
            *intents.entry(intent.clone()).or_insert(0) += 1;
        }
    }

    ProbeReport {
        distinct_miners: counts.len(),
        counts,
        failures,
        settled_asks,
        settled_units,
        intents,
    }
}

/// This function writes the records to disk, one JSON line per ask.
///
/// The file is what a later head-to-head analysis reads, so it holds
/// the fields that analysis needs and nothing about the payment beyond
/// the settlement transaction.
pub fn write_records(records: &[AskRecord], query: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(PROBE_PATH);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot make the probe directory: {error}"))?;
    }
    let mut file =
        fs::File::create(&path).map_err(|error| format!("cannot make the probe file: {error}"))?;
    for record in records {
        let line = serde_json::json!({
            "index": record.index,
            "query": query,
            "status": record.status,
            "settled": record.settled,
            "transaction": record.transaction,
            "miner_id": record.miner_id,
            "miner_name": record.miner_name,
            "intent": record.intent,
            "cost_usd": record.cost_usd,
            "signal_hash": record.signal_hash,
            "authorized_units": record.authorized_units,
            "failure": record.failure,
        });
        writeln!(file, "{line}")
            .map_err(|error| format!("cannot write the probe file: {error}"))?;
    }
    Ok(path)
}

/// This function prints the report and gives the gate verdict.
///
/// The function returns `true` when head-to-head data is obtainable
/// this way, which means more than one miner served the fixed query.
pub fn print_report(report: &ProbeReport, asked: usize, path: &Path) -> bool {
    println!();
    println!("=== ROUTING DISTRIBUTION, {asked} ASKS OF ONE FIXED QUERY ===");
    println!();
    println!(
        "{:<28} {:>10} {:>8} {:>8}",
        "miner", "miner_id", "asks", "share"
    );
    let answered: usize = report.counts.values().sum();
    for (key, count) in &report.counts {
        let (name, id) = key.split_once('\t').unwrap_or((key.as_str(), "?"));
        let share = if answered == 0 {
            0.0
        } else {
            100.0 * (*count as f64) / (answered as f64)
        };
        println!("{name:<28} {id:>10} {count:>8} {share:>7.1}%");
    }
    if report.counts.is_empty() {
        println!("(no ask named a miner)");
    }

    println!();
    println!("intents the router chose:");
    for (intent, count) in &report.intents {
        println!("  {intent:<24} {count}");
    }

    println!();
    println!("asks sent:        {asked}");
    println!("answered:         {answered}");
    println!("failed:           {}", report.failures.len());
    for failure in &report.failures {
        println!(
            "  ask {:>2}: {} (status {})",
            failure.index,
            failure.failure.as_deref().unwrap_or("unknown"),
            failure.status
        );
    }

    println!();
    println!("settled asks:     {}", report.settled_asks);
    println!(
        "settled spend:    {} smallest units ({:.2} USDC)",
        report.settled_units,
        (report.settled_units as f64) / 1_000_000.0
    );
    println!("record written:   {}", path.display());

    println!();
    println!("=== GATE ===");
    let open = report.distinct_miners > 1;
    if open {
        println!(
            "PASS: {} distinct miners served the same query.",
            report.distinct_miners
        );
        println!("Head-to-head data IS obtainable this way.");
    } else {
        println!(
            "STOP: {} distinct miner served all {answered} answered asks.",
            report.distinct_miners
        );
        println!("A fixed query does not spread over miners, so asking more of");
        println!("the same question cannot produce a head-to-head pair. The wave");
        println!("stops here rather than spending the rest of the budget.");
    }
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This helper builds a record without a network call.
    fn record(index: usize, miner: Option<(&str, &str)>, settled: bool) -> AskRecord {
        AskRecord {
            index,
            status: if miner.is_some() { 200 } else { 502 },
            settled,
            transaction: settled.then(|| "0xabc".to_string()),
            miner_id: miner.map(|(_, id)| id.to_string()),
            miner_name: miner.map(|(name, _)| name.to_string()),
            intent: miner.map(|_| "WEATHER_CHECK".to_string()),
            cost_usd: Some(0.01),
            signal_hash: None,
            authorized_units: 10000,
            failure: if miner.is_some() {
                None
            } else {
                Some("http 502".to_string())
            },
            body: String::new(),
        }
    }

    #[test]
    fn one_miner_over_every_ask_closes_the_gate() {
        let records: Vec<AskRecord> = (1..=30)
            .map(|index| record(index, Some(("zeus", "18")), true))
            .collect();
        let report = summarise(&records);
        assert_eq!(report.distinct_miners, 1);
        assert_eq!(report.counts["zeus\t18"], 30);
    }

    #[test]
    fn two_miners_open_the_gate() {
        let mut records: Vec<AskRecord> = (1..=20)
            .map(|index| record(index, Some(("zeus", "18")), true))
            .collect();
        records.extend((21..=30).map(|index| record(index, Some(("WeatherAPI", "212")), true)));
        let report = summarise(&records);
        assert_eq!(report.distinct_miners, 2);
        assert_eq!(report.counts["WeatherAPI\t212"], 10);
    }

    #[test]
    fn a_failed_ask_is_not_counted_as_a_miner() {
        let records = vec![
            record(1, Some(("zeus", "18")), true),
            record(2, None, false),
            record(3, Some(("zeus", "18")), true),
        ];
        let report = summarise(&records);
        assert_eq!(report.distinct_miners, 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.counts["zeus\t18"], 2);
    }

    #[test]
    fn a_settled_ask_that_failed_still_counts_into_the_spend() {
        // The money is gone whether or not an answer came back. A
        // report that hid this would understate what was spent.
        let records = vec![record(1, None, true)];
        let report = summarise(&records);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.settled_units, 10000);
        assert_eq!(report.settled_asks, 1);
    }

    #[test]
    fn an_unsettled_ask_adds_nothing_to_the_spend() {
        let records = vec![record(1, Some(("zeus", "18")), false)];
        let report = summarise(&records);
        assert_eq!(report.settled_units, 0);
        assert_eq!(report.settled_asks, 0);
    }

    #[test]
    fn a_two_hundred_with_no_miner_is_recorded_as_a_failure() {
        // A 200 that names no miner would otherwise vanish: not a
        // failure, and not a row in the distribution either.
        let outcome = AskOutcome {
            status: 200,
            body: r#"{"result":{"temp_c":28.1}}"#.to_string(),
            authorized_units: 10000,
            settlement: None,
        };
        let parsed = AskRecord::from_outcome(1, &outcome);
        assert_eq!(parsed.failure.as_deref(), Some("the answer names no miner"));
        assert!(parsed.miner_key().is_none());
    }

    #[test]
    fn a_real_answer_body_is_read_into_a_record() {
        let outcome = AskOutcome {
            status: 200,
            body: r#"{"miner_id":"212","miner_name":"WeatherAPI","cost_usd":0.01,
                     "intent":"WEATHER_CHECK","signal_hash":"0x3f"}"#
                .to_string(),
            authorized_units: 10000,
            settlement: None,
        };
        let parsed = AskRecord::from_outcome(7, &outcome);
        assert_eq!(parsed.index, 7);
        assert!(parsed.failure.is_none());
        assert_eq!(parsed.miner_name.as_deref(), Some("WeatherAPI"));
        assert_eq!(parsed.miner_key().as_deref(), Some("WeatherAPI\t212"));
        assert_eq!(parsed.cost_usd, Some(0.01));
        assert_eq!(parsed.intent.as_deref(), Some("WEATHER_CHECK"));
        // No settlement header means no proof of a spend.
        assert!(!parsed.settled);
    }
}
