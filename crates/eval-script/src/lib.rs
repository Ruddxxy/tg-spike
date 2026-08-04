//! # eval-script
//!
//! This crate is the canonical score script for Track 2 of the
//! Telegraph Protocol hackathon spike. A blockchain validator loads
//! this crate as a WASM module. The validator calls the exported
//! functions with a ground truth value and a miner response value.
//! The module returns a score.
//!
//! ## Score direction
//!
//! A high score is good. A low score is bad. The value 1.0 means a
//! perfect answer. The value 0.0 means the worst answer. Every
//! failure also returns 0.0. Bad input must never score better than
//! a wrong but well formed answer.
//!
//! This direction comes from the protocol rules, not from this
//! crate. Telegraph whitepaper v1.0 section 7.4 gives the router
//! formula `P(route to Miner_m) = Score_m / SUM(Score_j)`. The
//! router sends traffic in proportion to the score. Traffic must
//! flow to a good miner, so a high score must mean a good miner.
//! Whitepaper v1.0 section 4.3 names a score above 0.70 as a case
//! that may overscore a miner. Overscore means the score sits too
//! high for the miner's real quality. A score can only sit "too
//! high" if a high score means a good result. The protocol team
//! also states that a miner which does not answer "scores zero for
//! that round". Zero is a punishment, so zero must be the worst
//! score, not the best score.
//!
//! ## Why determinism matters here
//!
//! Every validator that runs this module on the same ground truth
//! and response pair must get back the exact same bit pattern. A
//! validator that computes a different score than the rest of the
//! network gets slashed. So this crate avoids every source of
//! non-determinism it can:
//! - No `HashMap` or `HashSet`. This crate uses `BTreeMap` where it
//!   needs an ordered table, because a `HashMap` does not promise a
//!   fixed iteration order across hosts.
//! - No clock, no random number generator, no file system, and no
//!   network access.
//! - No std transcendental math function such as `ln`, `log`,
//!   `exp`, or `powf` on the scored path. See the `math` module for
//!   the full reason and for the hand written `ln` this crate uses
//!   instead.
//! - A NaN value never reaches an exported function's return value.
//!   NaN bit patterns are not the same on every WASM host.
//!
//! ## Input size cap
//!
//! The host places no cap on a miner response size. This crate
//! enforces its own cap, `MAX_INPUT_BYTES`, so a huge miner
//! response cannot make a validator do unbounded work. The `abi`
//! module checks this cap before it reads any byte from linear
//! memory. See `MAX_INPUT_BYTES` for the reason every validator
//! must use the same cap value.
//!
//! ## Safety
//!
//! The functions this crate exports never panic and never trap. The
//! workspace release profile sets `panic = "abort"`. A panic in
//! that mode is a WASM trap, and a trap would stop the validator
//! that runs the module. So every function in this crate checks its
//! input before it uses the input, instead of relying on a panic
//! and a catch. See the `abi` module for the exported functions and
//! for the raw memory bound checks.
//!
//! ## Module layout
//!
//! - `abi`: the exported `alloc`, `dealloc`, `score`,
//!   `score_log_loss`, and `score_batch` functions, plus the raw
//!   memory bound checks and the input size cap check.
//! - `math`: the hand written `ln` function, Kahan summation, and
//!   the total order sort used for the batch mean.
//! - `metrics`: the Brier score, the log loss score, and the batch
//!   mean score.
//! - `parse`: the JSON parsing and field validation.
//! - `error`: the `ScoreError` type shared by every fallible
//!   function in this crate.

#![deny(missing_docs)]

pub mod abi;
pub mod error;
pub mod math;
pub mod metrics;
pub mod parse;

pub use error::ScoreError;

/// The largest input that the score functions will read, in bytes.
///
/// A validator gives a pointer and a length for each input. The
/// host places no cap on a miner response size, so this crate must
/// defend itself. The `abi` module checks this cap before it reads
/// any byte from linear memory. An input over the cap scores the
/// worst score, 0.0, at once.
///
/// This cap is a consensus-relevant constant. Every validator must
/// use the same cap value. A validator with a different cap could
/// score the same input differently than the rest of the network,
/// and that validator would get slashed for disagreeing with
/// consensus.
pub const MAX_INPUT_BYTES: u32 = 1_048_576; // 1 MiB

// These checks run at build time, not at run time. A build fails if
// either check fails. Both checks guard `MAX_INPUT_BYTES` itself, so
// they must sit next to the constant.
//
// The first check stops a future edit from setting the cap to 0. A
// cap of 0 would reject every input, including a well formed empty
// batch, and that is not what "a byte cap" means.
const _: () = assert!(MAX_INPUT_BYTES > 0);

// The second check proves the `u32` to `usize` round trip never
// throws bits away. The `abi` module casts a checked length from
// `u32` to `usize` to build a byte slice. That cast is lossless on
// every target this crate builds for today, because `usize` is at
// least 32 bits wide everywhere the crate runs. This assertion turns
// a silent truncation at the ABI boundary into a build failure, if a
// future edit ever changes the type of `MAX_INPUT_BYTES` or moves
// this crate to a target where the cast can lose bits.
const _: () = assert!(MAX_INPUT_BYTES as usize as u32 == MAX_INPUT_BYTES);
