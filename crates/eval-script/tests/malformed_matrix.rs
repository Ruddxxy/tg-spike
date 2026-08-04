//! This test drives the pure Rust `eval_script` API with every
//! malformed input shape the hard safety contract lists. Every case
//! here must produce an error from the pure API, which the ABI
//! layer in `eval_script::abi` then turns into the worst score,
//! 0.0. Two cases about a raw pointer and length pair, which the
//! pure `&[u8]` API cannot express, run through the real
//! `eval_script::abi::score` export instead. Those two calls never
//! read a real memory address, so they stay safe to run on a native
//! target. See the `abi` module doc comment for why a native target
//! cannot safely round-trip a real pointer through this ABI.

use eval_script::abi;
use eval_script::metrics::brier_from_bytes;

const VALID_GT: &[u8] = b"{\"label\": 1}";
const VALID_RESP: &[u8] = b"{\"confidence\": 0.5}";

#[test]
fn empty_input_is_an_error() {
    assert!(brier_from_bytes(b"", VALID_RESP).is_err());
    assert!(brier_from_bytes(VALID_GT, b"").is_err());
}

#[test]
fn non_utf8_bytes_are_an_error() {
    let bad = &[0xff, 0xfe, 0xfd];
    assert!(brier_from_bytes(bad, VALID_RESP).is_err());
    assert!(brier_from_bytes(VALID_GT, bad).is_err());
}

#[test]
fn invalid_json_is_an_error() {
    assert!(brier_from_bytes(b"{not json}", VALID_RESP).is_err());
    assert!(brier_from_bytes(VALID_GT, b"{not json}").is_err());
}

#[test]
fn missing_field_is_an_error() {
    assert!(brier_from_bytes(b"{}", VALID_RESP).is_err());
    assert!(brier_from_bytes(VALID_GT, b"{}").is_err());
}

#[test]
fn wrong_json_type_is_an_error() {
    assert!(brier_from_bytes(b"[1, 2]", VALID_RESP).is_err());
    assert!(brier_from_bytes(b"{\"label\": \"1\"}", VALID_RESP).is_err());
    assert!(brier_from_bytes(VALID_GT, b"{\"confidence\": \"high\"}").is_err());
    assert!(brier_from_bytes(VALID_GT, b"{\"confidence\": null}").is_err());
}

#[test]
fn label_not_in_zero_or_one_is_an_error() {
    assert!(brier_from_bytes(b"{\"label\": 2}", VALID_RESP).is_err());
    assert!(brier_from_bytes(b"{\"label\": -1}", VALID_RESP).is_err());
    assert!(brier_from_bytes(b"{\"label\": 0.5}", VALID_RESP).is_err());
}

#[test]
fn confidence_infinity_is_an_error() {
    // A JSON text cannot spell NaN or Infinity directly, but a
    // decimal exponent far past the f64 range parses to an infinite
    // float, or to a parse error. Either result must be an error
    // here.
    assert!(brier_from_bytes(VALID_GT, b"{\"confidence\": 1e400}").is_err());
    assert!(brier_from_bytes(VALID_GT, b"{\"confidence\": -1e400}").is_err());
}

#[test]
fn confidence_out_of_range_is_an_error() {
    assert!(brier_from_bytes(VALID_GT, b"{\"confidence\": -0.0001}").is_err());
    assert!(brier_from_bytes(VALID_GT, b"{\"confidence\": 1.0001}").is_err());
    assert!(brier_from_bytes(VALID_GT, b"{\"confidence\": 100}").is_err());
}

#[test]
fn a_pointer_and_length_pair_outside_memory_scores_the_worst_score() {
    // ptr + len overflows a u32. `abi::score` catches this before it
    // reads any memory and returns the worst score, 0.0.
    assert_eq!(abi::score(u32::MAX, u32::MAX, 0, 0), 0.0);
    assert_eq!(abi::score(0, 0, u32::MAX, u32::MAX), 0.0);
}

#[test]
fn an_integer_overflow_in_ptr_plus_len_scores_the_worst_score() {
    assert_eq!(abi::score(u32::MAX - 5, 10, 0, 0), 0.0);
    assert_eq!(abi::score_log_loss(u32::MAX - 5, 10, 0, 0), 0.0);
    assert_eq!(abi::score_batch(u32::MAX - 5, 10), 0.0);
}

#[test]
fn a_null_pointer_with_a_nonzero_length_scores_the_worst_score() {
    // ptr is 0 (null) but len is nonzero, so the offset does not
    // name a real block. `abi::score` returns the worst score
    // before it ever dereferences the pointer.
    assert_eq!(abi::score(0, 8, 0, 0), 0.0);
    assert_eq!(abi::score(0, 0, 0, 8), 0.0);
}

#[test]
fn empty_input_bytes_score_the_worst_score() {
    // Zero length input is valid UTF-8 (the empty string), but it
    // is not valid JSON. `abi::score_batch` never dereferences `ptr`
    // here, because `len` is 0.
    assert_eq!(abi::score_batch(0, 0), 0.0);
}

#[test]
fn an_oversize_length_scores_the_worst_score_without_a_trap() {
    // gt_len is one byte over `MAX_INPUT_BYTES`. gt_ptr is a wild
    // address that is never a valid block. `abi::score` checks the
    // length cap before it checks bounds or reads memory, so this
    // never touches the wild pointer.
    let over_cap = eval_script::MAX_INPUT_BYTES + 1;
    assert_eq!(abi::score(0xdead_beef, over_cap, 0, 0), 0.0);
    assert_eq!(abi::score_batch(0xdead_beef, over_cap), 0.0);
}
