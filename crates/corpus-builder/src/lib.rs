//! `corpus-builder` as a library.
//!
//! The binary in `main.rs` builds `corpus/weather-triples.jsonl`. This
//! library exposes the parts that a later wave needs to read that file
//! back and work with it.
//!
//! Only the modules that a consumer needs are public here. The fetch
//! and cache modules stay private to the binary, because a consumer
//! must never re-fetch: the corpus file is the record.
//!
//! `schema` is the important one. It holds the normaliser that turns
//! one raw miner response into a list of (valid time, Celsius) points.
//! The scoring wave needs that same normaliser, because the corpus
//! stores the FULL upstream response for each row, while the protocol
//! gives `rank_answer` a single extracted value. The evaluation tool
//! must therefore do the extraction itself, and it must do it with the
//! same code the corpus was built with. A second copy of that logic
//! would drift.

/// The rounding rule for a ground-truth temperature.
pub mod rounding;
/// The three miner response schemas and the shared normaliser.
pub mod schema;
/// Time helpers shared by the builder and the evaluator.
pub mod time;
