//! This module holds the error type for the score functions.
//!
//! Every variant maps to the worst score, 0.0, at the ABI boundary.
//! Bad input must never score better than a wrong but well formed
//! answer.

use crate::MAX_INPUT_BYTES;
use core::fmt;

/// This type lists the ways that a parse or a score step can fail.
///
/// `abi::finish` catches every `ScoreError` and turns it into the
/// worst score, 0.0, so no variant can reach a caller as anything but
/// 0.0. Rust code inside the crate can match on the variant to find
/// the exact cause.
///
/// Every variant below is constructed somewhere. An earlier version of
/// this crate scored a JSON label against a JSON confidence, and six
/// variants belonged to that model: `InvalidJson`, `MissingField`,
/// `WrongType`, `OutOfRange`, `InvalidLabel` and `InvalidConfidence`.
/// The model is gone, both inputs are now short texts holding one value
/// each, and there is no JSON to parse or field to look up. Those six
/// were removed rather than left as failures that cannot occur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreError {
    /// The input bytes are not valid UTF-8 text.
    InvalidUtf8,
    /// A pointer and length pair points outside the linear memory.
    BadPointer,
    /// A pointer plus a length overflows a 32-bit integer.
    PointerOverflow,
    /// The memory allocator could not give a block of memory.
    AllocFailed,
    /// The input is bigger than the byte cap this crate reads.
    ///
    /// See `MAX_INPUT_BYTES` for the cap value. The crate checks
    /// this cap before it reads any byte from linear memory, so an
    /// oversize miner response cannot make a validator do unbounded
    /// work.
    InputTooLarge,
}

impl fmt::Display for ScoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScoreError::InvalidUtf8 => {
                write!(f, "the input bytes are not valid UTF-8 text")
            }
            ScoreError::BadPointer => {
                write!(
                    f,
                    "the pointer and length pair points outside linear memory"
                )
            }
            ScoreError::PointerOverflow => {
                write!(f, "the pointer plus the length overflows a 32-bit integer")
            }
            ScoreError::AllocFailed => {
                write!(f, "the memory allocator could not give a block of memory")
            }
            ScoreError::InputTooLarge => {
                write!(f, "the input is bigger than the {MAX_INPUT_BYTES} byte cap")
            }
        }
    }
}

// `core` has no `Error` trait on a stable compiler, so this crate
// does not implement it. The type still implements `Display`, which
// is what a host-side caller needs to print a reason.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_display_message() {
        let variants = [
            ScoreError::InvalidUtf8,
            ScoreError::BadPointer,
            ScoreError::PointerOverflow,
            ScoreError::AllocFailed,
            ScoreError::InputTooLarge,
        ];
        for variant in variants {
            let text = variant.to_string();
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn input_too_large_message_names_the_cap() {
        let text = ScoreError::InputTooLarge.to_string();
        assert!(text.contains(&MAX_INPUT_BYTES.to_string()));
    }
}
