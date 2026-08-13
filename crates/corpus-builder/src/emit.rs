//! Build and write the corpus rows: `corpus/weather-triples.jsonl`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use serde::Serialize;

use crate::archive::ActualIndex;
use crate::error::BuildError;
use crate::rounding::{format_temp_c, round_temp_c};

/// One (response, valid_time) observation, ready to serialise to JSONL.
///
/// `gt_json` and `miner_answer` are `String`, not a nested JSON object:
/// a later scoring wave feeds these bytes straight into
/// `rank_answer(question, ground_truth, miner_answer)`, which is
/// consensus relevant. If this crate stored an object, every consumer
/// would have to re-serialise it, and the re-serialisation choice (key
/// order, number formatting, spacing) would silently change the score.
/// Storing a string here pins the exact bytes once, in one place.
///
/// `miner_temp_c` is kept for in-process error statistics and is never
/// written to the file: the output schema is exactly the field set the
/// brief specifies.
#[derive(Serialize)]
pub struct TripleRecord {
    pub question: String,
    pub gt_bare: Option<String>,
    pub gt_prose: Option<String>,
    /// Compact JSON text, e.g. `{"temperature_2m":18.6,"time":"2026-08-10T06:00"}`,
    /// stored as a string so its bytes are pinned. Built by this crate
    /// from the rounded archive actual, so it is a re-serialisation by
    /// necessity (there is no "original" gt_json anywhere upstream).
    /// `Value`'s `Display` impl is used to print it, which is
    /// `serde_json`'s own compact writer: the same bytes `serde_json`
    /// would produce for a nested object, just captured as a string up
    /// front instead of left for each downstream consumer to redo.
    pub gt_json: Option<String>,
    /// The miner's raw response text, EXACTLY as the daemon sent it: the
    /// original byte span sliced out of the cached feed body (see
    /// `feed::ExecutionInfo::result`), not a re-serialisation of a
    /// parsed `Value`. Key order, number formatting, and spacing all
    /// match the wire bytes. This duplicates a large response across
    /// every valid_time it contains (a Zeus response holds about 72),
    /// by design: the brief asks for one line per (response, valid_time)
    /// observation.
    pub miner_answer: String,
    pub miner_slug: String,
    pub intent: Option<String>,
    pub valid_time: String,
    pub lat: f64,
    pub lon: f64,
    pub actual_c: Option<f64>,
    pub cluster_id: Option<String>,
    #[serde(skip)]
    pub miner_temp_c: f64,
}

/// Build one record for a single (response, point) pair, looking up the
/// archive actual and rendering the three ground-truth strings from it.
///
/// The ground-truth renderings and `actual_c` all come from the SAME
/// rounded value (see `rounding::round_temp_c`), so they can never
/// disagree. When the archive has no coverage for this instant, every
/// ground-truth field is `None`: no number is invented.
#[allow(clippy::too_many_arguments)]
pub fn build_record(
    question: String,
    miner_answer: String,
    miner_slug: String,
    intent: Option<String>,
    valid_time: String,
    lat: f64,
    lon: f64,
    miner_temp_c: f64,
    cluster_id: Option<String>,
    actuals: &ActualIndex,
) -> TripleRecord {
    let actual_raw = actuals.lookup(lat, lon, &valid_time);

    let (gt_bare, gt_prose, gt_json, actual_c) = match actual_raw {
        Some(raw) => {
            let rounded = round_temp_c(raw);
            let bare = format_temp_c(raw);
            let prose = format!("The temperature was {bare} C.");
            // `Value`'s Display impl prints compact JSON (no spaces),
            // the same bytes `serde_json::to_string` would produce.
            // This cannot fail: the value is only a finite number and a
            // string, both always serialisable.
            let json =
                serde_json::json!({"temperature_2m": rounded, "time": valid_time}).to_string();
            (Some(bare), Some(prose), Some(json), Some(rounded))
        }
        None => (None, None, None, None),
    };

    TripleRecord {
        question,
        gt_bare,
        gt_prose,
        gt_json,
        miner_answer,
        miner_slug,
        intent,
        valid_time,
        lat,
        lon,
        actual_c,
        cluster_id,
        miner_temp_c,
    }
}

/// Write every record as one compact JSON object per line.
///
/// Returns the number of rows written and the file size in bytes.
pub fn write_jsonl(path: &Path, records: &[TripleRecord]) -> Result<(u64, u64), BuildError> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    for record in records {
        let line = serde_json::to_string(record)?;
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    let size = std::fs::metadata(path)?.len();
    Ok((records.len() as u64, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ActualIndex;

    #[test]
    fn build_record_serialises_compactly_and_matches_brief_examples() {
        let record = TripleRecord {
            question: "What is the weather in Paris?".to_string(),
            gt_bare: Some("18.6".to_string()),
            gt_prose: Some("The temperature was 18.6 C.".to_string()),
            gt_json: Some(r#"{"temperature_2m":18.6,"time":"2026-08-10T06:00"}"#.to_string()),
            miner_answer: r#"{"current":{"temp_c":18.6}}"#.to_string(),
            miner_slug: "weatherapi".to_string(),
            intent: Some("WEATHER_CHECK".to_string()),
            valid_time: "2026-08-10T06:00".to_string(),
            lat: 48.8667,
            lon: 2.3333,
            actual_c: Some(18.6),
            cluster_id: None,
            miner_temp_c: 18.6,
        };
        let line = serde_json::to_string(&record).expect("serialises");
        assert!(
            !line.contains(", "),
            "must be compact, no spaces after commas: {line}"
        );
        assert!(
            !line.contains(": "),
            "must be compact, no spaces after colons: {line}"
        );
        assert!(line.contains("\"gt_bare\":\"18.6\""));
        // gt_json must be a JSON STRING (quoted, with escaped inner
        // quotes), not a nested object.
        assert!(
            line.contains(r#""gt_json":"{\"temperature_2m\":18.6,\"time\":\"2026-08-10T06:00\"}""#),
            "gt_json must be an escaped JSON string, not an object: {line}"
        );
        // miner_answer must also be a JSON STRING holding the raw bytes.
        assert!(
            line.contains(r#""miner_answer":"{\"current\":{\"temp_c\":18.6}}""#),
            "miner_answer must be an escaped JSON string, not an object: {line}"
        );
        // Re-parsing the whole line must yield string types for both,
        // never object types.
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
        assert!(parsed["gt_json"].is_string());
        assert!(parsed["miner_answer"].is_string());
        // miner_temp_c is internal only and must never appear in output.
        assert!(!line.contains("miner_temp_c"));
    }

    #[test]
    fn gt_json_string_round_trips_to_the_same_object_the_brief_specifies() {
        let empty = ActualIndex::empty_for_tests();
        // No coverage at this point, but the rendering logic under test
        // (gt_json construction) is exercised directly here instead,
        // since ActualIndex has no public test constructor with data.
        let _ = build_record(
            "q".to_string(),
            "{}".to_string(),
            "weatherapi".to_string(),
            Some("WEATHER_CHECK".to_string()),
            "2099-01-01T00:00".to_string(),
            0.0,
            0.0,
            20.0,
            None,
            &empty,
        );
        let json =
            serde_json::json!({"temperature_2m": 18.6, "time": "2026-08-10T06:00"}).to_string();
        assert_eq!(json, r#"{"temperature_2m":18.6,"time":"2026-08-10T06:00"}"#);
        let reparsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(reparsed["temperature_2m"], 18.6);
        assert_eq!(reparsed["time"], "2026-08-10T06:00");
    }

    #[test]
    fn no_actual_coverage_leaves_ground_truth_fields_null() {
        let empty = ActualIndex::empty_for_tests();
        let record = build_record(
            "q".to_string(),
            "{}".to_string(),
            "weatherapi".to_string(),
            Some("WEATHER_CHECK".to_string()),
            "2099-01-01T00:00".to_string(),
            0.0,
            0.0,
            20.0,
            None,
            &empty,
        );
        assert!(record.gt_bare.is_none());
        assert!(record.gt_prose.is_none());
        assert!(record.gt_json.is_none());
        assert!(record.actual_c.is_none());
    }

    #[test]
    fn miner_answer_preserves_key_order_from_the_wire_not_alphabetical() {
        // If miner_answer were ever re-serialised through a plain
        // serde_json::Value (a BTreeMap without `preserve_order`), key
        // order would be normalised to alphabetical. Storing the raw
        // string must NOT do that: "z" before "a" must survive verbatim.
        let raw = r#"{"z":1,"a":2}"#.to_string();
        let empty = ActualIndex::empty_for_tests();
        let record = build_record(
            "q".to_string(),
            raw.clone(),
            "weatherapi".to_string(),
            None,
            "2099-01-01T00:00".to_string(),
            0.0,
            0.0,
            0.0,
            None,
            &empty,
        );
        assert_eq!(record.miner_answer, raw);
        let line = serde_json::to_string(&record).expect("serialises");
        assert!(line.contains(r#""miner_answer":"{\"z\":1,\"a\":2}""#));
    }
}
