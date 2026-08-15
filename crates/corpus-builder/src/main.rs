//! `corpus-builder`: fetch the Telegraph daemon question feed, normalise
//! the three weather-miner response schemas, join each observation to an
//! Open-Meteo archive actual, cluster paraphrased questions across
//! miners, and emit `corpus/weather-triples.jsonl` for `corpus-eval`
//! to consume.
//!
//! This is a HOST-SIDE tool: plain std, native, no wasm, no scoring
//! logic. Run it from the workspace root so the relative `corpus/`
//! output path resolves correctly:
//!
//! ```text
//! cargo run -p corpus-builder            # normal run, uses/builds the cache
//! cargo run -p corpus-builder -- --refresh   # force a full re-fetch
//! cargo run -p corpus-builder -- --offline   # fail on any cache miss
//! ```

mod archive;
mod cache;
mod city;
mod cluster;
mod emit;
mod error;
mod feed;
mod rounding;
mod schema;
mod time;

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use error::BuildError;
use feed::QuestionEntry;
use schema::{WeatherPoint, WEATHER_MINER_SLUGS};

/// True if a feed row's `execution.result` is JSON null.
///
/// `execution.result` is stored as the raw original bytes (see
/// `feed::ExecutionInfo`), so checking null means parsing that slice
/// back into a `Value`. That slice came from JSON this crate itself just
/// parsed, so a parse failure here should not happen; if it somehow
/// does, it is treated the same as null (never a panic).
fn result_is_null(entry: &QuestionEntry) -> bool {
    serde_json::from_str::<Value>(entry.execution.result.get())
        .map(|v| v.is_null())
        .unwrap_or(true)
}

/// Base URL for the daemon question feed (query string added per page).
const FEED_BASE_URL: &str = "https://devnode.telegraphprotocol.com/daemon/api/questions";

/// Root output directory this crate owns.
const CORPUS_DIR: &str = "corpus";

struct Args {
    refresh: bool,
    offline: bool,
}

fn parse_args() -> Args {
    let mut refresh = false;
    let mut offline = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--refresh" => refresh = true,
            "--offline" => offline = true,
            other => eprintln!("warning: ignoring unrecognised argument {other}"),
        }
    }
    Args { refresh, offline }
}

/// One weather-miner response that normalised successfully, with
/// everything the later stages need, all owned (no borrows into the
/// original feed page).
struct SurvivedResponse {
    row_id: String,
    miner_slug: String,
    intent: Option<String>,
    question_text: String,
    /// The raw miner response text, exactly as the daemon sent it (see
    /// `feed::ExecutionInfo::result`). Stored as a `String`, not a
    /// parsed `Value`, so it can go straight into the corpus with no
    /// re-serialisation step to introduce a different byte encoding.
    miner_answer: String,
    points: Vec<WeatherPoint>,
    city: Option<&'static str>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("corpus-builder failed: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), BuildError> {
    let args = parse_args();
    let cache_dir = Path::new(CORPUS_DIR).join("cache");
    let mut cache = cache::HttpCache::open(&cache_dir, args.refresh, args.offline)?;

    println!("=== SUBTASK 1: fetch and cache ===");
    let (all_rows, total) = feed::fetch_all(&mut cache, FEED_BASE_URL)?;
    println!(
        "Fetched {} feed rows (server-reported total={total}). Network requests so far: {}",
        all_rows.len(),
        cache.network_requests()
    );

    println!();
    println!("=== SUBTASK 2: normalise ===");
    let weather_rows: Vec<&QuestionEntry> = all_rows
        .iter()
        .filter(|r| {
            r.routing
                .miner_slug
                .as_deref()
                .is_some_and(|s| WEATHER_MINER_SLUGS.contains(&s))
        })
        .collect();
    println!("Weather-miner rows in feed: {}", weather_rows.len());

    let mut drop_counts: HashMap<&'static str, u64> = HashMap::new();
    let mut null_result_status_counts: HashMap<String, u64> = HashMap::new();
    let mut survived: Vec<SurvivedResponse> = Vec::new();

    for entry in &weather_rows {
        let miner_slug = entry.routing.miner_slug.clone().unwrap_or_default();
        let raw_result_text = entry.execution.result.get();
        // Parse the raw bytes back into a Value for the normalisers,
        // which need real field access, not just a byte slice. A parse
        // failure here is treated as null (never a panic); see
        // `result_is_null`.
        let parsed_result: Value = serde_json::from_str(raw_result_text).unwrap_or(Value::Null);
        if parsed_result.is_null() {
            *null_result_status_counts
                .entry(entry.status.clone())
                .or_insert(0) += 1;
        }
        match schema::normalise(&miner_slug, &parsed_result) {
            Ok(points) => {
                let city = city::extract_city(&entry.question.text);
                survived.push(SurvivedResponse {
                    row_id: entry.id.clone(),
                    miner_slug,
                    intent: entry.routing.intent.clone(),
                    question_text: entry.question.text.clone(),
                    miner_answer: raw_result_text.to_string(),
                    points,
                    city,
                });
            }
            Err(reason) => {
                *drop_counts.entry(reason.label()).or_insert(0) += 1;
            }
        }
    }

    println!(
        "Responses normalised successfully: {} of {}",
        survived.len(),
        weather_rows.len()
    );
    println!("Drop reasons:");
    let mut drop_list: Vec<_> = drop_counts.iter().collect();
    drop_list.sort();
    for (reason, count) in drop_list {
        println!("  {reason}: {count}");
    }
    let null_total: u64 = null_result_status_counts.values().sum();
    println!("Null-result rows observed (regardless of status): {null_total}, by status: {null_result_status_counts:?}");

    println!();
    println!("=== SUBTASK 3: ground truth (Open-Meteo archive lookup) ===");
    let all_points: Vec<&WeatherPoint> = survived.iter().flat_map(|r| r.points.iter()).collect();
    println!(
        "Total normalised points needing an archive lookup: {}",
        all_points.len()
    );
    let actuals = archive::build_index(&mut cache, &all_points)?;
    println!(
        "Archive location groups with a genuine fetch failure: {}",
        actuals.failed_groups.len()
    );
    for (key, reason) in &actuals.failed_groups {
        println!("  group {key:?}: {reason}");
    }

    println!();
    println!("=== SUBTASK 4: paraphrase clusters ===");
    let mut min_max: Vec<(String, String)> = Vec::with_capacity(survived.len());
    for r in &survived {
        let mut min_t = r.points[0].valid_time_utc.clone();
        let mut max_t = r.points[0].valid_time_utc.clone();
        for p in &r.points {
            if p.valid_time_utc < min_t {
                min_t = p.valid_time_utc.clone();
            }
            if p.valid_time_utc > max_t {
                max_t = p.valid_time_utc.clone();
            }
        }
        min_max.push((min_t, max_t));
    }

    let mut no_city_count = 0u64;
    let mut city_but_no_intent_count = 0u64;
    let mut cluster_inputs = Vec::new();
    for (i, r) in survived.iter().enumerate() {
        match (r.city, r.intent.as_deref()) {
            (Some(city), Some(intent)) => {
                cluster_inputs.push(cluster::ClusterInput {
                    row_id: &r.row_id,
                    miner_slug: &r.miner_slug,
                    city,
                    intent,
                    min_time: &min_max[i].0,
                    max_time: &min_max[i].1,
                });
            }
            (None, _) => no_city_count += 1,
            (Some(_), None) => city_but_no_intent_count += 1,
        }
    }
    println!("Responses with no extractable city (excluded from clustering): {no_city_count}");
    println!("Responses with a city but no intent (excluded from clustering): {city_but_no_intent_count}");

    let (assignment, clusters) = cluster::build_clusters(&cluster_inputs);
    let multi_miner: Vec<&cluster::Cluster> = clusters
        .iter()
        .filter(|c| c.distinct_miner_count() >= 2)
        .collect();
    println!(
        "Total clusters formed (including size-1): {}",
        clusters.len()
    );
    println!("Clusters with 2+ distinct miners: {}", multi_miner.len());
    for c in &multi_miner {
        println!(
            "  {} (city={}, intent={}) : {} miners = {:?}",
            c.id,
            c.city,
            c.intent,
            c.distinct_miner_count(),
            c.miner_slugs
        );
    }

    println!();
    println!("=== SUBTASK 5: emit corpus ===");
    let mut records = Vec::new();
    let mut errors_by_miner: HashMap<String, Vec<f64>> = HashMap::new();
    let mut rows_by_intent: HashMap<String, u64> = HashMap::new();
    let mut rows_by_miner: HashMap<String, u64> = HashMap::new();
    let mut rows_with_actual = 0u64;
    let mut rows_without_actual = 0u64;

    for r in &survived {
        let cluster_id = assignment.get(&r.row_id).cloned();
        for p in &r.points {
            let record = emit::build_record(
                r.question_text.clone(),
                r.miner_answer.clone(),
                r.miner_slug.clone(),
                r.intent.clone(),
                p.valid_time_utc.clone(),
                p.lat,
                p.lon,
                p.temp_c,
                cluster_id.clone(),
                &actuals,
            );
            *rows_by_intent
                .entry(record.intent.clone().unwrap_or_else(|| "NONE".to_string()))
                .or_insert(0) += 1;
            *rows_by_miner.entry(record.miner_slug.clone()).or_insert(0) += 1;
            if let Some(actual) = record.actual_c {
                errors_by_miner
                    .entry(record.miner_slug.clone())
                    .or_default()
                    .push((record.miner_temp_c - actual).abs());
                rows_with_actual += 1;
            } else {
                rows_without_actual += 1;
            }
            records.push(record);
        }
    }

    let output_path = Path::new(CORPUS_DIR).join("weather-triples.jsonl");
    let (row_count, file_size) = emit::write_jsonl(&output_path, &records)?;
    println!(
        "Wrote {row_count} rows to {} ({file_size} bytes = {:.1} MB)",
        output_path.display(),
        file_size as f64 / (1024.0 * 1024.0)
    );

    println!();
    println!("Rows by intent:");
    let mut by_intent: Vec<_> = rows_by_intent.iter().collect();
    by_intent.sort();
    for (k, n) in by_intent {
        println!("  {k}: {n}");
    }

    println!("Rows by miner:");
    let mut by_miner: Vec<_> = rows_by_miner.iter().collect();
    by_miner.sort();
    for (k, n) in by_miner {
        println!("  {k}: {n}");
    }

    println!("Rows with an archive actual: {rows_with_actual}");
    println!("Rows with NO archive actual (future-dated or coverage gap), excluded from error stats: {rows_without_actual}");

    println!();
    println!("Per-miner absolute error vs archive actuals (Celsius):");
    let mut miners: Vec<_> = errors_by_miner.keys().cloned().collect();
    miners.sort();
    for miner in miners {
        let mut errs = errors_by_miner[&miner].clone();
        errs.sort_by(f64::total_cmp);
        let n = errs.len();
        let mean = errs.iter().sum::<f64>() / n as f64;
        let median = percentile(&errs, 50.0);
        let p90 = percentile(&errs, 90.0);
        println!("  {miner}: n={n} mean={mean:.3} median={median:.3} p90={p90:.3}");
    }

    print_non_hour_boundary_diagnostic(&records, &actuals);

    print_known_bad_rows(&all_rows, &weather_rows, &survived, &assignment, &clusters);

    println!();
    println!("=== RUN SUMMARY ===");
    println!(
        "Total network requests this run: {}",
        cache.network_requests()
    );

    Ok(())
}

/// Compute the nearest-rank percentile of an ascending-sorted slice by
/// linear interpolation, rounded to the nearest index.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len().saturating_sub(1))]
}

/// Measure how many rows have no archive actual purely because their
/// `valid_time` is not on an exact hour boundary (WeatherAPI `current`
/// observations land at `:15` or `:30`; the archive is hourly).
///
/// This is a MEASUREMENT ONLY: this crate does not snap a valid_time to
/// the nearest hour anywhere. It reports what an exact-timestamp string
/// join loses, so a reader can decide whether that loss matters.
fn print_non_hour_boundary_diagnostic(
    records: &[emit::TripleRecord],
    actuals: &archive::ActualIndex,
) {
    println!();
    println!("=== Non-hour-boundary valid_time diagnostic ===");

    let mut total = 0u64;
    let mut null_count = 0u64;
    let mut recoverable_at_exact_hour = 0u64;

    for record in records {
        if record.valid_time.ends_with(":00") {
            continue;
        }
        total += 1;
        if record.actual_c.is_none() {
            null_count += 1;
            // "YYYY-MM-DDTHH:MM" -> "YYYY-MM-DDTHH:00": same date and
            // hour, minute forced to zero, to check whether the archive
            // has that hour at all.
            if record.valid_time.len() >= 13 {
                let hour_truncated = format!("{}:00", &record.valid_time[..13]);
                if actuals
                    .lookup(record.lat, record.lon, &hour_truncated)
                    .is_some()
                {
                    recoverable_at_exact_hour += 1;
                }
            }
        }
    }

    println!("Rows whose valid_time is not on an exact hour boundary: {total}");
    println!("  of those, actual_c is null: {null_count}");
    println!(
        "  of those nulls, the archive DOES have data for the same hour truncated to :00 (lost purely to exact-timestamp matching, not a coverage gap): {recoverable_at_exact_hour}"
    );
}

fn print_known_bad_rows(
    all_rows: &[QuestionEntry],
    weather_rows: &[&QuestionEntry],
    survived: &[SurvivedResponse],
    assignment: &HashMap<String, String>,
    clusters: &[cluster::Cluster],
) {
    println!();
    println!("=== Known-bad rows ===");

    // 1. Alpha Vantage ETH / Ethan Allen ticker mismatch.
    match all_rows
        .iter()
        .find(|r| r.question.text.contains("price of Ethereum right now"))
    {
        Some(bad1) => {
            let slug = bad1.routing.miner_slug.as_deref().unwrap_or("<none>");
            let is_weather_miner = WEATHER_MINER_SLUGS.contains(&slug);
            println!("1. Ethereum/Ethan-Allen row FOUND in feed: miner_slug={slug:?} intent={:?} is_weather_miner={is_weather_miner}", bad1.routing.intent);
            println!("   This pipeline filters to the three weather miner slugs before normalising anything, so it NEVER sees this row: a weather-only pipeline is structurally blind to CRYPTO_PRICE rows, not just uninterested in them.");
        }
        None => println!("1. Ethereum/Ethan-Allen row NOT FOUND in this feed snapshot."),
    }

    // 2. Maringá mis-geocode (WeatherAPI resolved "Brazil, Indiana, USA").
    match survived.iter().find(|r| r.question_text.contains("aring")) {
        Some(bad2) => {
            let coords = bad2.points.first().map(|p| (p.lat, p.lon));
            println!("2. Maringá row FOUND, normalised successfully. miner_slug={} extracted city label={:?}, but the response's own lat/lon={coords:?} (Indiana, USA, not Brazil).", bad2.miner_slug, bad2.city);
            println!("   NOT caught: this crate never cross-checks a miner's returned location against the question's city. The archive lookup silently uses the wrong (Indiana) coordinates and can still produce a real-looking actual_c there.");
        }
        None => println!("2. Maringá row not found among surviving weather responses."),
    }

    // 3. OpenWeatherMap Miami "October 10-15 2022" wrong-date forecast.
    match survived.iter().find(|r| r.question_text.contains("Miami Climate")) {
        Some(bad3) => {
            let times: Vec<&str> = bad3.points.iter().map(|p| p.valid_time_utc.as_str()).collect();
            let min_t = times.iter().min().copied().unwrap_or("?");
            let max_t = times.iter().max().copied().unwrap_or("?");
            println!("3. Miami OpenWeatherMap 'October 10-15 2022' row FOUND, normalised successfully. Its {} points span {min_t} .. {max_t} — real dates from this run, not October 2022.", bad3.points.len());
            println!("   NOT caught: this crate never parses a date out of question text, so it cannot tell the returned dates don't match the question. Those dates usually fall inside the archive's coverage window, so actual_c gets computed and looks like ordinary valid data.");
            match assignment.get(&bad3.row_id) {
                Some(cid) => {
                    let miners: Vec<&str> = clusters.iter().find(|c| &c.id == cid).map(|c| c.miner_slugs.iter().map(String::as_str).collect()).unwrap_or_default();
                    let survives_as_multi_miner = miners.len() >= 2;
                    println!("   It belongs to cluster {cid} (miners={miners:?}). This cluster survives as a multi-miner cluster on a semantically wrong row: {survives_as_multi_miner}. Nothing in the (city, intent, time-overlap) clustering rule can distinguish a real date from a wrong one.");
                }
                None => println!("   It was not assigned to any cluster (no city and/or intent match), so this known-bad row does not affect a multi-miner cluster."),
            }
        }
        None => println!("3. Miami OpenWeatherMap 'October 10-15 2022' row not found among surviving weather responses."),
    }

    // 4. WeatherAPI Lisbon null result, status "error".
    let lisbon_null = weather_rows
        .iter()
        .any(|r| r.question.text.contains("Lisbon") && result_is_null(r));
    let lisbon_status: Vec<&str> = weather_rows
        .iter()
        .filter(|r| r.question.text.contains("Lisbon") && result_is_null(r))
        .map(|r| r.status.as_str())
        .collect();
    println!("4. Lisbon null-result row: present in feed = {lisbon_null}, observed status = {lisbon_status:?}.");
    println!("   Caught: the null-result check runs before any schema dispatch, regardless of status, so this row is excluded with reason 'null_result'.");

    // 5. OpenWeatherMap resolved "the moon" (not a city) to a real place
    // in Iran named "Moon".
    match survived.iter().find(|r| r.question_text.contains("upper stage impact the moon")) {
        Some(bad5) => {
            let coords = bad5.points.first().map(|p| (p.lat, p.lon));
            let valid_time = bad5.points.first().map(|p| p.valid_time_utc.as_str());
            println!("5. \"Will upper stage impact the moon on August 5?\" row FOUND, normalised successfully. miner_slug={} extracted city label={:?}, but OpenWeatherMap resolved this non-city question to a real place named \"Moon\" at lat/lon={coords:?} (Iran), valid_time={valid_time:?}.", bad5.miner_slug, bad5.city);
            println!("   NOT caught: same failure class as Maringá (known-bad row 2). This crate has no concept of \"is this question even about a place\", so a miner confidently resolving a non-entity to a real-sounding location passes straight through and can still get a real-looking actual_c.");
        }
        None => println!("5. \"Will upper stage impact the moon on August 5?\" row not found among surviving weather responses."),
    }
}
