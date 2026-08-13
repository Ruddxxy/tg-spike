//! This crate is the miner simulator for the Telegraph Track 2 spike.
//!
//! The crate makes synthetic miners with a known true quality. It scores
//! each miner with the compiled `eval-script` WASM module. It then
//! checks that the leaderboard agrees with the quality that the crate
//! built in.
//!
//! ## Why this crate exists
//!
//! A scoring rule is only correct if it ranks a good miner above a bad
//! miner. This crate makes the quality order known in advance. The
//! crate can then find a scoring rule that gets the order wrong. The
//! same harness works against the real Canonical Script of the protocol
//! when the protocol publishes it.
//!
//! ## Rules this crate keeps
//!
//! - The crate scores every miner through the WASM boundary. It never
//!   scores a response with native Rust code. Only the compiled module
//!   tells the truth about a validator. The published ABI has one
//!   scoring entry point, `rank_answer`, and this crate calls only that
//!   one. An earlier version of this crate also scored a native,
//!   probabilistic log loss metric; that metric tested a confidence
//!   field the real ABI does not have, so it is gone, not ported.
//! - The crate uses its own PRNG. It does not use the `rand` crate. The
//!   same seed gives the same output on every machine, forever.
//! - The crate does not change `eval-script`. That crate is the system
//!   under test.

#![deny(missing_docs)]

pub mod archetype;
pub mod bootstrap;
pub mod dataset;
pub mod leaderboard;
pub mod rng;
pub mod scoring;
pub mod types;
pub mod verdict;

pub use types::{
    Archetype, Dataset, DatasetShape, Item, LeaderboardRow, Metric, MinerResult, Response,
    ResponseKind, VerdictLine,
};
