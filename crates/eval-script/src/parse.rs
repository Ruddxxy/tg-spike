//! This module reads JSON bytes and builds typed values.
//!
//! The module uses `serde_json` to parse the JSON text. `serde_json`
//! compiles into this module as pure Rust code. It does not use a
//! host import. This keeps the parse step the same on every WASM
//! host. `serde_json` also has a correctly rounded float parser. A
//! hand written parser would not give that same guarantee.

use crate::error::ScoreError;
use serde_json::Value;

/// This struct holds a parsed ground truth label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroundTruth {
    /// The true label. The value is 0 or 1.
    pub label: u8,
}

/// This struct holds a parsed miner response.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Response {
    /// The confidence value. The range is 0.0 to 1.0.
    pub confidence: f64,
}

/// This function turns raw bytes into a JSON value.
///
/// The function checks that the bytes are valid UTF-8 text first.
/// The function then checks that the text is valid JSON.
fn parse_json(bytes: &[u8]) -> Result<Value, ScoreError> {
    let text = core::str::from_utf8(bytes).map_err(|_| ScoreError::InvalidUtf8)?;
    serde_json::from_str(text).map_err(|_| ScoreError::InvalidJson)
}

/// This function reads the field map out of a JSON value.
///
/// The function returns an error if the value is not a JSON object.
fn as_object(value: &Value) -> Result<&serde_json::Map<String, Value>, ScoreError> {
    value.as_object().ok_or(ScoreError::WrongType("root"))
}

/// This function parses ground truth bytes into a `GroundTruth`.
///
/// The input must be a JSON object with a `label` field. The
/// `label` field must be the JSON integer 0 or the JSON integer 1.
/// The function ignores any other field in the object.
pub fn parse_ground_truth(bytes: &[u8]) -> Result<GroundTruth, ScoreError> {
    let value = parse_json(bytes)?;
    ground_truth_from_value(&value)
}

/// This function reads a `GroundTruth` out of an already parsed
/// JSON value.
///
/// The batch score function uses this function. The batch score
/// function already has a parsed `Value` for each pair. This
/// function lets the batch score function skip a second parse of
/// the same JSON text.
pub fn ground_truth_from_value(value: &Value) -> Result<GroundTruth, ScoreError> {
    let object = as_object(value)?;
    let label_value = object
        .get("label")
        .ok_or(ScoreError::MissingField("label"))?;
    let label_number = label_value.as_i64().ok_or(ScoreError::WrongType("label"))?;
    match label_number {
        0 => Ok(GroundTruth { label: 0 }),
        1 => Ok(GroundTruth { label: 1 }),
        _ => Err(ScoreError::InvalidLabel),
    }
}

/// This function parses response bytes into a `Response`.
///
/// The input must be a JSON object with a `confidence` field. The
/// `confidence` field must be a finite JSON number. The value must
/// sit in the range 0.0 to 1.0. The function ignores any other
/// field in the object.
pub fn parse_response(bytes: &[u8]) -> Result<Response, ScoreError> {
    let value = parse_json(bytes)?;
    response_from_value(&value)
}

/// This function reads a `Response` out of an already parsed JSON
/// value. See `ground_truth_from_value` for the reason this
/// function exists.
pub fn response_from_value(value: &Value) -> Result<Response, ScoreError> {
    let object = as_object(value)?;
    let confidence_value = object
        .get("confidence")
        .ok_or(ScoreError::MissingField("confidence"))?;
    let confidence = confidence_value
        .as_f64()
        .ok_or(ScoreError::WrongType("confidence"))?;
    if !confidence.is_finite() {
        return Err(ScoreError::InvalidConfidence);
    }
    if !(0.0..=1.0).contains(&confidence) {
        return Err(ScoreError::OutOfRange("confidence"));
    }
    Ok(Response { confidence })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_ground_truth() {
        assert_eq!(parse_ground_truth(b"{\"label\": 0}").unwrap().label, 0);
        assert_eq!(parse_ground_truth(b"{\"label\": 1}").unwrap().label, 1);
    }

    #[test]
    fn rejects_bad_label_value() {
        assert!(parse_ground_truth(b"{\"label\": 2}").is_err());
        assert!(parse_ground_truth(b"{\"label\": -1}").is_err());
        assert!(parse_ground_truth(b"{\"label\": 0.5}").is_err());
        assert!(parse_ground_truth(b"{\"label\": \"0\"}").is_err());
    }

    #[test]
    fn rejects_missing_label_field() {
        assert!(parse_ground_truth(b"{}").is_err());
    }

    #[test]
    fn ignores_extra_fields() {
        let text = b"{\"label\": 1, \"source\": \"oracle\"}";
        assert_eq!(parse_ground_truth(text).unwrap().label, 1);
    }

    #[test]
    fn parses_whitespace_padded_json() {
        let text = b"  {  \"label\" : 0 }  ";
        assert_eq!(parse_ground_truth(text).unwrap().label, 0);
    }

    #[test]
    fn rejects_non_utf8_bytes() {
        assert!(parse_ground_truth(&[0xff, 0xfe, 0x00]).is_err());
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse_ground_truth(b"not json").is_err());
        assert!(parse_ground_truth(b"").is_err());
    }

    #[test]
    fn parses_valid_response() {
        let response = parse_response(b"{\"confidence\": 0.75}").unwrap();
        assert!((response.confidence - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn parses_integer_confidence() {
        let zero = parse_response(b"{\"confidence\": 0}").unwrap();
        let one = parse_response(b"{\"confidence\": 1}").unwrap();
        assert_eq!(zero.confidence, 0.0);
        assert_eq!(one.confidence, 1.0);
    }

    #[test]
    fn rejects_out_of_range_confidence() {
        assert!(parse_response(b"{\"confidence\": -0.1}").is_err());
        assert!(parse_response(b"{\"confidence\": 1.1}").is_err());
    }

    #[test]
    fn rejects_extreme_confidence_values() {
        // A very large decimal exponent gives an infinite float or
        // a parse error. Either way the result must be an error.
        assert!(parse_response(b"{\"confidence\": 1e400}").is_err());
        assert!(parse_response(b"{\"confidence\": -1e400}").is_err());
    }

    #[test]
    fn rejects_wrong_type_confidence() {
        assert!(parse_response(b"{\"confidence\": \"high\"}").is_err());
        assert!(parse_response(b"{\"confidence\": null}").is_err());
    }

    #[test]
    fn rejects_missing_confidence_field() {
        assert!(parse_response(b"{}").is_err());
    }

    #[test]
    fn rejects_non_object_root() {
        assert!(parse_ground_truth(b"[1, 2, 3]").is_err());
        assert!(parse_response(b"\"just a string\"").is_err());
    }
}
