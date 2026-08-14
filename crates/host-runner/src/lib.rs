//! This crate is the host runner for the Telegraph Track 2 canonical
//! script. It stands in for a blockchain validator. It loads the compiled
//! `eval-script` WASM module and drives its memory ABI through the
//! `wasmtime` crate.
//!
//! The published ABI has three wasm exports: `alloc`, `dealloc`, and
//! `rank_answer`. Every score this crate reports from the `.wasm`
//! module comes from `rank_answer`, called through `wasmtime`, the same
//! way a validator would call it. This crate makes no native, non-wasm
//! score of any kind. The `MAX_INPUT_BYTES` constant is the one
//! exception to "wasm only": it is re-exported from `eval-script`
//! through [`cases::MAX_INPUT_BYTES`], because a real validator has
//! only the `.wasm` binary and cannot import a Rust constant the way
//! this workspace does. See that item for the full reason.
//!
//! This crate also holds the wasmtime vs wazero cross host check. See
//! [`golden`] for the shared JSON result shape and [`cross_host`] for
//! the bit equality assertion.

pub mod cases;
pub mod checks;
pub mod cross_host;
pub mod display;
pub mod golden;
pub mod instance;
