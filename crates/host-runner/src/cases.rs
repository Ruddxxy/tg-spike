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
    /// The question text to send. It may be empty, and it may be
    /// junk; the module must not require it.
    #[serde(default)]
    pub question: String,
    /// The ground truth text to send. Send the bytes as they are.
    pub ground_truth: String,
    /// The miner answer text to send. Send the bytes as they are.
    pub miner_answer: String,
    /// The expected score. Compare with bit equality, not `==`.
    pub expected: f64,
}

/// This is the shape of the whole `golden_vectors.json` file.
#[derive(Debug, Deserialize)]
struct GoldenFile {
    vectors: Vec<GoldenVector>,
}

/// The environment variable that overrides the golden vector file.
pub const GOLDEN_VECTORS_ENV: &str = "TG_GOLDEN_VECTORS";

/// This finds the path to `golden_vectors.json` at the workspace root.
///
/// The file sits next to the workspace `Cargo.toml`, two levels up from
/// this crate's manifest directory.
///
/// [`GOLDEN_VECTORS_ENV`] overrides it. That exists for the tolerance
/// bands: the expected bit patterns in the root file are calibrated for
/// `TOLERANCE = 0.03`, so a `price` or `onchain` module cannot match
/// them and must be checked against its own file. The override is a
/// test-harness affordance only. Nothing in the shipped `.wasm` reads
/// an environment variable, and the module imports nothing.
pub fn golden_vectors_path() -> PathBuf {
    if let Ok(path) = std::env::var(GOLDEN_VECTORS_ENV) {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
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

/// This builds the malformed input matrix.
///
/// Every case must not trap, and every case must return exactly 0.0,
/// the worst score.
///
/// The matrix changed with the new scoring model. The old model parsed
/// both sides as JSON, so a text such as `{label: 0` was a PARSE
/// ERROR and scored 0.0. Both sides are now free text, so that same
/// text is simply a text that holds no value, and it scores by its
/// token overlap. It is no longer a worst-score case, so it is no
/// longer in this matrix.
///
/// What remains are the cases that must still reach exactly 0.0:
/// - a blank miner answer, in every whitespace form,
/// - a miner answer that is not valid UTF-8,
/// - an input over [`MAX_INPUT_BYTES`], which the cap rejects before
///   any byte is read,
/// - an answer that shares no token with the ground truth.
pub fn malformed_cases() -> Vec<MalformedCase> {
    let mut cases = vec![
        MalformedCase {
            id: 1,
            name: "empty miner answer, valid ground truth",
            ground_truth: b"192.43".to_vec(),
            response: b"".to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 2,
            name: "whitespace only miner answer",
            ground_truth: b"192.43".to_vec(),
            response: b"   \t\n\r ".to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 3,
            name: "both sides empty",
            ground_truth: b"".to_vec(),
            response: b"".to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 4,
            name: "non-UTF-8 bytes in the miner answer",
            ground_truth: b"192.43".to_vec(),
            response: vec![0xff, 0xfe, 0xfd],
            expect_worst_score: true,
        },
        MalformedCase {
            id: 5,
            name: "control characters only in the miner answer",
            ground_truth: b"192.43".to_vec(),
            response: vec![0x00, 0x01, 0x02],
            expect_worst_score: true,
        },
        MalformedCase {
            id: 6,
            name: "a word that shares no token with the ground truth",
            ground_truth: b"malicious".to_vec(),
            response: b"sunny".to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 7,
            name: "a number against an incompatible unit",
            ground_truth: b"307.85 K".to_vec(),
            response: b"307.85 USD".to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 8,
            name: "a negated answer against a plain ground truth",
            ground_truth: b"malicious".to_vec(),
            response: b"not malicious".to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 9,
            name: "the NaN word is not a number",
            ground_truth: b"192.43".to_vec(),
            response: b"NaN".to_vec(),
            expect_worst_score: true,
        },
        MalformedCase {
            id: 10,
            name: "the Infinity word is not a number",
            ground_truth: b"192.43".to_vec(),
            response: b"Infinity".to_vec(),
            expect_worst_score: true,
        },
    ];

    // The cap cases. Each one is far over `MAX_INPUT_BYTES`, so the
    // cap rejects it before the host reads any byte from memory.
    let over_cap = (MAX_INPUT_BYTES as usize) + 1;

    cases.push(MalformedCase {
        id: 11,
        name: "huge miner answer, about 10 MB, valid-looking text, over the cap",
        ground_truth: b"192.43".to_vec(),
        response: {
            let mut payload = b"192.43 ".to_vec();
            payload.resize(10 * 1024 * 1024, b'a');
            payload
        },
        expect_worst_score: true,
    });

    cases.push(MalformedCase {
        id: 12,
        name: "huge miner answer, about 10 MB, pure garbage bytes, over the cap",
        ground_truth: b"192.43".to_vec(),
        response: (0..10 * 1024 * 1024).map(|i| (i % 256) as u8).collect(),
        expect_worst_score: true,
    });

    cases.push(MalformedCase {
        id: 13,
        name: "ground truth one byte over the cap",
        ground_truth: vec![b'x'; over_cap],
        response: b"192.43".to_vec(),
        expect_worst_score: true,
    });

    cases.push(MalformedCase {
        id: 14,
        name: "miner answer one byte over the cap",
        ground_truth: b"192.43".to_vec(),
        response: vec![b'x'; over_cap],
        expect_worst_score: true,
    });

    cases.push(MalformedCase {
        id: 15,
        name: "miner answer exactly at the cap, shares no token",
        ground_truth: b"192.43".to_vec(),
        response: vec![b'x'; MAX_INPUT_BYTES as usize],
        expect_worst_score: true,
    });

    cases
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
