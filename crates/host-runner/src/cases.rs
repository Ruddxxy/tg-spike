//! This module has the test data: golden vectors and the malformed input
//! matrix. It does not call wasmtime. It only builds and reads data.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

/// This is one golden vector row, read from `golden_vectors.json`.
#[derive(Debug, Deserialize)]
pub struct GoldenVector {
    /// The short name of the vector, for the printed table.
    pub name: String,
    /// The raw JSON bytes to send as ground truth. Send them as they are.
    pub ground_truth: String,
    /// The raw JSON bytes to send as the miner response. Send them as they
    /// are.
    pub response: String,
    /// The expected score. Compare with bit equality, not `==`.
    pub expected: f64,
}

/// This is the shape of the whole `golden_vectors.json` file.
#[derive(Debug, Deserialize)]
struct GoldenFile {
    vectors: Vec<GoldenVector>,
}

/// This finds the path to `golden_vectors.json` at the workspace root.
///
/// The file sits next to the workspace `Cargo.toml`, two levels up from
/// this crate's manifest directory.
pub fn golden_vectors_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("golden_vectors.json")
}

/// This reads and parses `golden_vectors.json` from the given path.
pub fn load_golden_vectors(path: &std::path::Path) -> Result<Vec<GoldenVector>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read golden vectors file at {}", path.display()))?;
    let file: GoldenFile = serde_json::from_str(&raw)
        .with_context(|| format!("cannot parse golden vectors file at {}", path.display()))?;
    Ok(file.vectors)
}

// NOTE on this import: a real Telegraph validator never has the
// eval-script source. It only has the compiled `.wasm` file. A real
// validator cannot import a Rust constant, so it must read
// `MAX_INPUT_BYTES` some other way, or keep a hand-copied number in
// sync with the script by hand. That is an open production question,
// not solved here.
//
// This workspace is different: `host-runner` and `eval-script` build
// from the same source tree at the same time, so importing the real
// constant is possible and removes a real risk. Before this import,
// this file held its own hardcoded copy of `MAX_INPUT_BYTES`. Two
// copies of one consensus-relevant number is a defect waiting to
// happen: a future edit could change one copy and not the other, and
// then validators built from this tree would disagree about which
// payloads are valid, which splits the stake-weighted median. This
// import makes that class of defect impossible inside this workspace.
// It does not fix the production question above.
/// The largest ground truth or response size, in bytes, that eval-script
/// reads before it checks the length.
///
/// This is a consensus-relevant constant. Every validator must use the
/// same cap. A `gt_len` or `resp_len` over this cap must return 0.0
/// right away, before the host reads any byte from linear memory. See
/// `eval_script::MAX_INPUT_BYTES` for the full doc comment on this
/// value.
pub use eval_script::MAX_INPUT_BYTES;

/// This is one row of the malformed input matrix.
pub struct MalformedCase {
    /// The case number, matching the spec list, starting at 1.
    pub id: u32,
    /// A short name for the case.
    pub name: &'static str,
    /// The raw ground truth bytes to send.
    pub ground_truth: Vec<u8>,
    /// The raw response bytes to send.
    pub response: Vec<u8>,
    /// True if this case must return exactly 0.0, the worst score, to
    /// pass.
    ///
    /// Every case in this matrix expects 0.0. A case gets there by one
    /// of two paths: its content fails to parse, or its size is over
    /// [`MAX_INPUT_BYTES`], so the cap rejects it before any byte is
    /// read. Read each case's `name` and inline comment to see which
    /// path it takes.
    pub expect_worst_score: bool,
}

/// This makes a valid ground truth payload for cases that only test the
/// response side.
fn valid_gt() -> Vec<u8> {
    br#"{"label": 1}"#.to_vec()
}

/// This makes a valid response payload for cases that only test the
/// ground truth side.
fn valid_resp() -> Vec<u8> {
    br#"{"confidence": 0.5}"#.to_vec()
}

/// This builds the full malformed input matrix, cases 1 through 27.
///
/// Every case must not trap. Every case must return exactly 0.0, the
/// worst score. Cases 23 through 27 test the [`MAX_INPUT_BYTES`] cap.
/// See the [`MalformedCase::expect_worst_score`] doc comment for the two
/// paths a case can take to reach 0.0.
pub fn malformed_cases() -> Vec<MalformedCase> {
    let mut cases = vec![
        MalformedCase {
            id: 1,
            name: "empty ground_truth, valid response",
            ground_truth: b"".to_vec(),
            response: valid_resp(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 2,
            name: "empty response, valid ground_truth",
            ground_truth: valid_gt(),
            response: b"".to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 3,
            name: "both empty",
            ground_truth: b"".to_vec(),
            response: b"".to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 4,
            name: "non-UTF-8 bytes in ground_truth",
            ground_truth: vec![0xff, 0xfe, 0xfd],
            response: valid_resp(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 5,
            name: "non-UTF-8 bytes in response",
            ground_truth: valid_gt(),
            response: vec![0xff, 0xfe, 0xfd],
            expect_worst_score: true,
        },
        MalformedCase {
            id: 6,
            name: "invalid JSON: unquoted key, no closing brace",
            ground_truth: b"{label: 0".to_vec(),
            response: valid_resp(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 7,
            name: "valid JSON but not an object",
            ground_truth: b"[1,2,3]".to_vec(),
            response: valid_resp(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 8,
            name: "missing field: ground_truth {}",
            ground_truth: b"{}".to_vec(),
            response: valid_resp(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 9,
            name: "missing field: response {}",
            ground_truth: valid_gt(),
            response: b"{}".to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 10,
            name: "wrong type: label is a string",
            ground_truth: br#"{"label": "one"}"#.to_vec(),
            response: valid_resp(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 11,
            name: "wrong type: confidence is a string",
            ground_truth: valid_gt(),
            response: br#"{"confidence": "high"}"#.to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 12,
            name: "wrong type: confidence is null",
            ground_truth: valid_gt(),
            response: br#"{"confidence": null}"#.to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 13,
            name: "label out of range: 2",
            ground_truth: br#"{"label": 2}"#.to_vec(),
            response: valid_resp(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 14,
            name: "label out of range: -1",
            ground_truth: br#"{"label": -1}"#.to_vec(),
            response: valid_resp(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 15,
            name: "confidence NaN token (invalid JSON)",
            ground_truth: valid_gt(),
            response: br#"{"confidence": NaN}"#.to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 16,
            name: "confidence +Infinity token (invalid JSON)",
            ground_truth: valid_gt(),
            response: br#"{"confidence": Infinity}"#.to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 17,
            name: "confidence -Infinity token (invalid JSON)",
            ground_truth: valid_gt(),
            response: br#"{"confidence": -Infinity}"#.to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 18,
            name: "confidence huge exponent overflows to +Inf",
            ground_truth: valid_gt(),
            response: br#"{"confidence": 1e400}"#.to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 19,
            name: "confidence huge negative exponent",
            ground_truth: valid_gt(),
            response: br#"{"confidence": -1e400}"#.to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 20,
            name: "confidence out of range low: -0.5",
            ground_truth: valid_gt(),
            response: br#"{"confidence": -0.5}"#.to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 21,
            name: "confidence out of range high: 1.5",
            ground_truth: valid_gt(),
            response: br#"{"confidence": 1.5}"#.to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 22,
            name: "deeply nested JSON, about 200 levels of '['",
            ground_truth: valid_gt(),
            response: "[".repeat(200).into_bytes(),
            expect_worst_score: true,
        },
    ];

    // These cases build their payload with a function call. They do not
    // fit in the `vec![]` literal above as plain field values, so they
    // are added here instead.
    cases.push(MalformedCase {
        id: 23,
        name: "huge payload, about 10 MB, valid-looking JSON, over the cap",
        ground_truth: valid_gt(),
        response: build_huge_valid_json(),
        // This payload has well-formed JSON content. Before the input
        // cap, that made this case not malformed; it got a real score.
        // Its size is far over `MAX_INPUT_BYTES`, so the cap now
        // rejects it before any byte is read. It returns 0.0, like
        // every other case here.
        expect_worst_score: true,
    });
    cases.push(MalformedCase {
        id: 24,
        name: "huge payload, about 10 MB, pure garbage bytes, over the cap",
        ground_truth: valid_gt(),
        response: build_huge_garbage(),
        // This payload is over `MAX_INPUT_BYTES`, so the cap rejects it
        // before any byte is read. It never reaches the JSON parser.
        expect_worst_score: true,
    });
    cases.push(MalformedCase {
        id: 25,
        name: "gt_len exactly at the 1 MiB cap, invalid JSON",
        // This size sits exactly at `MAX_INPUT_BYTES`. The cap does not
        // reject a size at the cap, only a size over it. This payload
        // reaches the JSON parser and fails to parse there, because it
        // is not a JSON object. It returns 0.0 through the parse-failure
        // path, not the cap path.
        ground_truth: vec![b'x'; MAX_INPUT_BYTES as usize],
        response: valid_resp(),
        expect_worst_score: true,
    });
    cases.push(MalformedCase {
        id: 26,
        name: "gt_len one byte over the 1 MiB cap",
        // This size is one byte over `MAX_INPUT_BYTES`. The cap rejects
        // it before any byte is read from linear memory.
        ground_truth: vec![b'x'; MAX_INPUT_BYTES as usize + 1],
        response: valid_resp(),
        expect_worst_score: true,
    });
    cases.push(MalformedCase {
        id: 27,
        name: "resp_len one byte over the 1 MiB cap",
        ground_truth: valid_gt(),
        // This size is one byte over `MAX_INPUT_BYTES`. The cap rejects
        // it before any byte is read from linear memory.
        response: vec![b'x'; MAX_INPUT_BYTES as usize + 1],
        expect_worst_score: true,
    });

    cases
}

/// This builds a valid-looking JSON payload, padded to about 10 MB.
///
/// It has a valid `confidence` field and a `pad` field with a long string.
/// This tests that the module handles a large but well-formed payload
/// without excess cost, and without a stack or memory problem.
fn build_huge_valid_json() -> Vec<u8> {
    const TARGET_SIZE: usize = 10 * 1024 * 1024;
    let prefix = br#"{"confidence": 0.5, "pad": ""#;
    let suffix = br#""}"#;
    let pad_len = TARGET_SIZE - prefix.len() - suffix.len();
    let mut out = Vec::with_capacity(TARGET_SIZE);
    out.extend_from_slice(prefix);
    out.resize(out.len() + pad_len, b'a');
    out.extend_from_slice(suffix);
    out
}

/// This builds about 10 MB of pure garbage bytes. It is not valid UTF-8
/// and not valid JSON.
fn build_huge_garbage() -> Vec<u8> {
    const TARGET_SIZE: usize = 10 * 1024 * 1024;
    let mut out = Vec::with_capacity(TARGET_SIZE);
    for i in 0..TARGET_SIZE {
        // This makes a repeating, non-UTF-8-friendly pattern. It is not
        // random. The test must stay repeatable across runs.
        out.push((i % 256) as u8);
    }
    // Force some high bytes in so this can never parse as UTF-8.
    for byte in out.iter_mut().step_by(7) {
        *byte = 0xff;
    }
    out
}

/// This truncates a byte slice for a printed table cell.
///
/// It shows the first `max_len` bytes, escaped, plus a note of the full
/// length when the input is longer than that.
pub fn truncate_for_display(bytes: &[u8], max_len: usize) -> String {
    let shown = &bytes[..bytes.len().min(max_len)];
    let text = String::from_utf8_lossy(shown);
    if bytes.len() > max_len {
        format!("{text:?} (truncated, full length {} bytes)", bytes.len())
    } else {
        format!("{text:?}")
    }
}
