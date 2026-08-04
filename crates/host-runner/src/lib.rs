//! This crate is the host runner for the Telegraph Track 2 canonical
//! script. It stands in for a blockchain validator. It loads the compiled
//! `eval-script` WASM module and drives its memory ABI through the
//! `wasmtime` crate.
//!
//! This crate never calls `eval-script` as native Rust code. Every score
//! comes from the compiled `.wasm` module, loaded from a file path, the
//! same way a validator would load it. The one exception is the
//! `MAX_INPUT_BYTES` constant, re-exported from `eval-script` through
//! [`cases::MAX_INPUT_BYTES`]. See that item for why a real validator
//! cannot do the same thing this workspace does.

pub mod cases;
pub mod checks;
pub mod instance;
