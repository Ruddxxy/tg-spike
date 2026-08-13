//! This module has the wasmtime vs wazero cross host bit equality
//! check. See the [`crate::golden`] module for the shared JSON shape
//! both hosts read and write.
//!
//! A wasmtime/wazero disagreement, on the same `.wasm` file and the
//! same golden vector, is a consensus relevant defect. The Telegraph
//! network can run either engine underneath its wasm host. If the two
//! engines score the same input differently, two honest validators on
//! different engines produce different Local Scores for the same
//! miner, and that splits the stake weighted median. This check exists
//! to catch that class of defect before it reaches a real network.
//!
//! The check asserts that the wasmtime side vector NAME SET and the
//! wazero side vector NAME SET are exactly equal, not only that every
//! wasmtime name has a matching wazero row. A name that exists only on
//! the wazero side is evidence the two runs did not score the same
//! golden vector file, or scored it with two different builds of this
//! crate. That gap must fail loudly, the same as a missing name on the
//! wazero side.

use std::collections::BTreeSet;
use std::path::Path;

use crate::golden::{self, GoldenResult};

/// This is the default path to the wazero side golden result file.
pub const DEFAULT_WAZERO_GOLDEN_PATH: &str = "target/golden-f32-wazero.json";

/// This is one compared golden vector, both hosts' bit pattern side by
/// side.
pub struct VectorComparison {
    /// The vector name.
    pub name: String,
    /// The wasmtime side `f32` bit pattern, as hex text.
    pub wasmtime_bits_hex: String,
    /// The wazero side `f32` bit pattern, as hex text. This reads
    /// `"(missing)"` when the wazero file has no row for this vector
    /// name.
    pub wazero_bits_hex: String,
    /// The wasmtime side score value.
    pub wasmtime_value: f64,
    /// The wazero side score value. This reads `f64::NAN` when the
    /// wazero file has no row for this vector name. `bit_match` is
    /// always `false` in that case, so a report must never print this
    /// field without also checking `bit_match`.
    pub wazero_value: f64,
    /// True if the two bit patterns match exactly.
    pub bit_match: bool,
}

/// This is the full outcome of the cross host check.
///
/// A missing file, a stale hash, and a bit mismatch are all first
/// class outcomes of an evidence gathering check, not host side
/// errors. [`compare_cross_host`] never panics and never returns an
/// `Err`; it returns one of these variants instead.
pub enum CrossHostOutcome {
    /// The wazero result file does not exist, or exists but does not
    /// parse as the shared JSON shape. `command` is the exact command
    /// line that produces a fresh, valid file.
    MissingFile {
        /// The command to run to produce the missing or broken file.
        command: String,
    },
    /// The wazero file's `wasm_sha256` does not match the hash of the
    /// `.wasm` file this run loaded. The file is stale evidence, from
    /// a different `.wasm` build.
    Stale {
        /// The SHA-256 hash of the `.wasm` file this run loaded.
        wasmtime_sha256: String,
        /// The SHA-256 hash recorded inside the wazero result file.
        wazero_sha256: String,
    },
    /// Both files loaded and the hashes matched. This holds one row
    /// per vector this run scored, plus the overall pass/fail.
    Compared {
        /// One row per compared vector.
        results: Vec<VectorComparison>,
        /// Vector names that the wazero file holds but this run's own
        /// wasmtime results do not name at all.
        ///
        /// This is not the same defect as a missing wazero row for a
        /// wasmtime vector; that case already shows up as a `(missing)`
        /// row in `results`. An EXTRA wazero-side name means the two
        /// files disagree about which golden vectors exist at all, so
        /// it must fail the check on its own, not be silently dropped
        /// because the comparison loop only walks the wasmtime side.
        /// The list is sorted, so a printed report is stable across
        /// runs.
        extra_wazero_names: Vec<String>,
        /// True only if every vector's bit pattern matched AND the two
        /// files name the exact same set of vectors.
        pass: bool,
    },
}

/// This gives the exact command line that produces a fresh wazero
/// golden result file for `wasm_path_display`, at `wazero_path`.
fn rebuild_command(wasm_path_display: &str, wazero_path: &Path) -> String {
    format!(
        "cd tools/wazero-runner && go run . -golden ../../golden_vectors.json \
         -a ../../{wasm_path_display} -out ../../{}",
        wazero_path.display()
    )
}

/// This runs the wasmtime vs wazero cross host bit equality check.
///
/// `wasmtime_results` is this run's own golden vector results.
/// `wasmtime_wasm_sha256` is the SHA-256 hash of the `.wasm` file this
/// run loaded. `wasm_path_display` names that `.wasm` file, for the
/// missing-file command hint. `wazero_path` is the path to the wazero
/// side result file, normally [`DEFAULT_WAZERO_GOLDEN_PATH`].
///
/// See [`CrossHostOutcome`] for the three failure cases plus the
/// success case.
pub fn compare_cross_host(
    wasmtime_results: &[GoldenResult],
    wasmtime_wasm_sha256: &str,
    wasm_path_display: &str,
    wazero_path: &Path,
) -> CrossHostOutcome {
    if !wazero_path.exists() {
        return CrossHostOutcome::MissingFile {
            command: rebuild_command(wasm_path_display, wazero_path),
        };
    }

    let wazero_file = match golden::load_golden_file(wazero_path) {
        Ok(file) => file,
        Err(_) => {
            // The file exists but does not parse as the shared JSON
            // shape. This is not stale evidence and not a bit
            // mismatch; there is no usable evidence at all. Treat it
            // the same as a missing file: the fix is the same
            // command, run again.
            return CrossHostOutcome::MissingFile {
                command: rebuild_command(wasm_path_display, wazero_path),
            };
        }
    };

    if wazero_file.wasm_sha256 != wasmtime_wasm_sha256 {
        return CrossHostOutcome::Stale {
            wasmtime_sha256: wasmtime_wasm_sha256.to_string(),
            wazero_sha256: wazero_file.wasm_sha256,
        };
    }

    let mut results = Vec::with_capacity(wasmtime_results.len());
    let mut pass = true;
    for wt in wasmtime_results {
        match wazero_file.vectors.iter().find(|wz| wz.name == wt.name) {
            Some(wz) => {
                let bit_match = wt.bits_hex == wz.bits_hex;
                pass &= bit_match;
                results.push(VectorComparison {
                    name: wt.name.clone(),
                    wasmtime_bits_hex: wt.bits_hex.clone(),
                    wazero_bits_hex: wz.bits_hex.clone(),
                    wasmtime_value: wt.value,
                    wazero_value: wz.value,
                    bit_match,
                });
            }
            None => {
                // The wazero file has no row for this vector name.
                // There is no evidence for this vector, so this must
                // not pass silently.
                pass = false;
                results.push(VectorComparison {
                    name: wt.name.clone(),
                    wasmtime_bits_hex: wt.bits_hex.clone(),
                    wazero_bits_hex: "(missing)".to_string(),
                    wasmtime_value: wt.value,
                    wazero_value: f64::NAN,
                    bit_match: false,
                });
            }
        }
    }

    // The loop above only walks `wasmtime_results`, so it can never see
    // a name that exists ONLY on the wazero side. Find that gap here,
    // by comparing the two full NAME SETS, not just looking up each
    // wasmtime name in turn. A `BTreeSet` gives a sorted, deterministic
    // order for the printed list.
    let wasmtime_names: BTreeSet<&str> =
        wasmtime_results.iter().map(|wt| wt.name.as_str()).collect();
    let extra_wazero_names: Vec<String> = wazero_file
        .vectors
        .iter()
        .map(|wz| wz.name.as_str())
        .filter(|name| !wasmtime_names.contains(name))
        .collect::<BTreeSet<&str>>()
        .into_iter()
        .map(str::to_string)
        .collect();
    if !extra_wazero_names.is_empty() {
        pass = false;
    }

    CrossHostOutcome::Compared {
        results,
        extra_wazero_names,
        pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::golden::golden_result;

    #[test]
    fn missing_file_gives_a_runnable_command() {
        let path = Path::new("target/this-file-does-not-exist-for-a-test.json");
        let outcome = compare_cross_host(&[], "abc123", "some/module.wasm", path);
        match outcome {
            CrossHostOutcome::MissingFile { command } => {
                assert!(command.contains("go run ."));
                assert!(command.contains("-golden"));
                assert!(command.contains("some/module.wasm"));
            }
            _ => panic!("expected MissingFile"),
        }
    }

    #[test]
    fn stale_hash_is_reported_with_both_hashes() {
        let dir = std::env::temp_dir().join(format!(
            "host-runner-cross-host-test-stale-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("cannot create temp dir for this test");
        let path = dir.join("wazero.json");
        golden::write_wasmtime_golden(&path, "a.wasm", "wazero-hash", &[])
            .expect("write must succeed");

        let outcome = compare_cross_host(&[], "wasmtime-hash", "a.wasm", &path);
        match outcome {
            CrossHostOutcome::Stale {
                wasmtime_sha256,
                wazero_sha256,
            } => {
                assert_eq!(wasmtime_sha256, "wasmtime-hash");
                assert_eq!(wazero_sha256, "wazero-hash");
            }
            _ => panic!("expected Stale"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn matching_hash_and_matching_bits_passes() {
        let dir = std::env::temp_dir().join(format!(
            "host-runner-cross-host-test-pass-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("cannot create temp dir for this test");
        let path = dir.join("wazero.json");
        let wazero_results = vec![golden_result("v1", 1.0_f32)];
        golden::write_wasmtime_golden(&path, "a.wasm", "same-hash", &wazero_results)
            .expect("write must succeed");

        let wasmtime_results = vec![golden_result("v1", 1.0_f32)];
        let outcome = compare_cross_host(&wasmtime_results, "same-hash", "a.wasm", &path);
        match outcome {
            CrossHostOutcome::Compared {
                results,
                extra_wazero_names,
                pass,
            } => {
                assert!(pass);
                assert_eq!(results.len(), 1);
                assert!(results[0].bit_match);
                assert!(extra_wazero_names.is_empty());
            }
            _ => panic!("expected Compared"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn matching_hash_and_differing_bits_fails() {
        let dir = std::env::temp_dir().join(format!(
            "host-runner-cross-host-test-fail-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("cannot create temp dir for this test");
        let path = dir.join("wazero.json");
        let wazero_results = vec![golden_result("v1", 1.0_f32)];
        golden::write_wasmtime_golden(&path, "a.wasm", "same-hash", &wazero_results)
            .expect("write must succeed");

        let wasmtime_results = vec![golden_result("v1", 0.5_f32)];
        let outcome = compare_cross_host(&wasmtime_results, "same-hash", "a.wasm", &path);
        match outcome {
            CrossHostOutcome::Compared {
                results,
                extra_wazero_names,
                pass,
            } => {
                assert!(!pass);
                assert!(!results[0].bit_match);
                assert!(extra_wazero_names.is_empty());
            }
            _ => panic!("expected Compared"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// This is the regression test for the defect this module used to
    /// have: the comparison loop only walked the wasmtime side, so a
    /// vector name that existed ONLY in the wazero file was silently
    /// dropped and never made the check fail. Here the wazero file
    /// names two vectors, `v1` and `v2`, but this run only scored `v1`.
    /// `v2` must show up in `extra_wazero_names`, by name, and the
    /// overall check must fail.
    #[test]
    fn extra_vector_on_the_wazero_side_fails_and_is_named() {
        let dir = std::env::temp_dir().join(format!(
            "host-runner-cross-host-test-extra-wazero-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("cannot create temp dir for this test");
        let path = dir.join("wazero.json");
        let wazero_results = vec![golden_result("v1", 1.0_f32), golden_result("v2", 0.5_f32)];
        golden::write_wasmtime_golden(&path, "a.wasm", "same-hash", &wazero_results)
            .expect("write must succeed");

        let wasmtime_results = vec![golden_result("v1", 1.0_f32)];
        let outcome = compare_cross_host(&wasmtime_results, "same-hash", "a.wasm", &path);
        match outcome {
            CrossHostOutcome::Compared {
                results,
                extra_wazero_names,
                pass,
            } => {
                assert!(!pass, "an extra wazero-side vector must fail the check");
                assert_eq!(results.len(), 1, "the wasmtime side scored one vector");
                assert!(results[0].bit_match, "the shared vector v1 still matches");
                assert_eq!(extra_wazero_names, vec!["v2".to_string()]);
            }
            _ => panic!("expected Compared"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// This is the mirror image of the test above: a vector name that
    /// exists only on the wasmtime side, missing from the wazero file.
    /// This path already worked before this change; this test locks it
    /// in so a future edit cannot silently drop it while fixing the
    /// extra-name gap.
    #[test]
    fn missing_vector_on_the_wazero_side_fails_and_is_named() {
        let dir = std::env::temp_dir().join(format!(
            "host-runner-cross-host-test-missing-wazero-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("cannot create temp dir for this test");
        let path = dir.join("wazero.json");
        let wazero_results = vec![golden_result("v1", 1.0_f32)];
        golden::write_wasmtime_golden(&path, "a.wasm", "same-hash", &wazero_results)
            .expect("write must succeed");

        let wasmtime_results = vec![golden_result("v1", 1.0_f32), golden_result("v2", 0.5_f32)];
        let outcome = compare_cross_host(&wasmtime_results, "same-hash", "a.wasm", &path);
        match outcome {
            CrossHostOutcome::Compared {
                results,
                extra_wazero_names,
                pass,
            } => {
                assert!(!pass, "a missing wazero-side vector must fail the check");
                assert_eq!(results.len(), 2);
                let v2 = results
                    .iter()
                    .find(|r| r.name == "v2")
                    .expect("v2 row must be present");
                assert_eq!(v2.wazero_bits_hex, "(missing)");
                assert!(!v2.bit_match);
                assert!(
                    extra_wazero_names.is_empty(),
                    "no wazero-only name exists in this case"
                );
            }
            _ => panic!("expected Compared"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
