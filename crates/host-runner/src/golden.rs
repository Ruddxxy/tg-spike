//! This module has the shared golden result JSON shape.
//!
//! A Go tool, `wazero-runner`, and this crate both score the same
//! golden vectors and write a result file in this shape, one file per
//! host. The [`crate::cross_host`] module reads both files and checks
//! that the two hosts agree, bit for bit. See
//! `tools/wazero-runner/golden.go` for the Go side of this shape. The
//! two shapes must stay field-for-field identical, or the JSON will
//! not compare cleanly.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// This is one golden vector result: the vector name, its `f32` bit
/// pattern as hex text, and its value as `f64`, for a human reader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenResult {
    /// The vector name, from `golden_vectors.json`.
    pub name: String,
    /// The `f32` bit pattern, as `0x` prefixed, lowercase, 8 digit hex
    /// text.
    pub bits_hex: String,
    /// The score value, widened from `f32` to `f64` so JSON holds it
    /// exactly.
    pub value: f64,
}

/// This is the whole golden result file: which runner made it, which
/// `.wasm` file it scored, that file's SHA-256 hash, and every vector
/// result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenOutput {
    /// The name of the host that made this file: `"wasmtime"` or
    /// `"wazero"`.
    pub runner: String,
    /// The `.wasm` file path, as given on that runner's command line.
    pub wasm_path: String,
    /// The lowercase hex SHA-256 hash of the `.wasm` file bytes.
    pub wasm_sha256: String,
    /// One result per golden vector.
    pub vectors: Vec<GoldenResult>,
}

/// This turns a vector name and an `f32` score into a [`GoldenResult`]
/// row.
pub fn golden_result(name: &str, value: f32) -> GoldenResult {
    GoldenResult {
        name: name.to_string(),
        bits_hex: format!("0x{:08x}", value.to_bits()),
        value: f64::from(value),
    }
}

/// This gives the lowercase hex SHA-256 hash of a byte slice.
///
/// This matches the hash `wazero-runner` writes with Go's
/// `crypto/sha256` and `encoding/hex`. Both hosts hash the same raw
/// `.wasm` file bytes, so a matching hash proves the two runs scored
/// the same build.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// This writes the wasmtime side golden result file.
///
/// The shape matches the Go `wazero-runner` tool's golden mode output,
/// with `"runner": "wasmtime"` in place of `"wazero"`. It creates any
/// missing parent directory before it writes the file.
pub fn write_wasmtime_golden(
    out_path: &Path,
    wasm_path_display: &str,
    wasm_sha256: &str,
    results: &[GoldenResult],
) -> Result<()> {
    let output = GoldenOutput {
        runner: "wasmtime".to_string(),
        wasm_path: wasm_path_display.to_string(),
        wasm_sha256: wasm_sha256.to_string(),
        vectors: results.to_vec(),
    };
    let json = serde_json::to_string_pretty(&output)
        .context("cannot encode the wasmtime golden result as JSON")?;
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create directory {}", parent.display()))?;
        }
    }
    std::fs::write(out_path, format!("{json}\n"))
        .with_context(|| format!("cannot write golden result file to {}", out_path.display()))?;
    Ok(())
}

/// This reads and parses a golden result file from disk.
pub fn load_golden_file(path: &Path) -> Result<GoldenOutput> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read golden result file at {}", path.display()))?;
    let output: GoldenOutput = serde_json::from_str(&raw)
        .with_context(|| format!("cannot parse golden result file at {}", path.display()))?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_a_known_vector() {
        // The SHA-256 hash of the empty byte string is a well known
        // constant. Any correct implementation must match it.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn golden_result_bits_hex_has_eight_lowercase_hex_digits_with_prefix() {
        let result = golden_result("one", 1.0_f32);
        assert_eq!(result.bits_hex, "0x3f800000");
        assert_eq!(result.value, 1.0);
    }

    #[test]
    fn write_then_load_round_trips_every_field() {
        let dir =
            std::env::temp_dir().join(format!("host-runner-golden-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("cannot create temp dir for this test");
        let path = dir.join("golden-round-trip.json");

        let results = vec![golden_result("a", 0.5_f32), golden_result("b", 1.0_f32)];
        write_wasmtime_golden(&path, "some/path.wasm", "deadbeef", &results)
            .expect("write must succeed");

        let loaded = load_golden_file(&path).expect("load must succeed");
        assert_eq!(loaded.runner, "wasmtime");
        assert_eq!(loaded.wasm_path, "some/path.wasm");
        assert_eq!(loaded.wasm_sha256, "deadbeef");
        assert_eq!(loaded.vectors.len(), 2);
        assert_eq!(loaded.vectors[0].bits_hex, "0x3f000000");
        assert_eq!(loaded.vectors[1].bits_hex, "0x3f800000");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
