//! This module compares two short texts.
//!
//! The module normalises each text, splits it into tokens, and scores
//! the overlap. It runs with no heap allocation, so it works in a
//! `no_std` build.
//!
//! ## Why the score divides by the union
//!
//! The baseline scorer divides the overlap count by the token count of
//! the MINER answer. That choice pays a miner for saying less. A miner
//! that answers the single word "is" against the ground truth "is
//! malicious" gets an overlap of 1 and a miner token count of 1, so it
//! scores 1.0 for a word that carries no information.
//!
//! This module divides by the size of the union of both token sets.
//! The same attack now gives 1 shared token out of 2 total tokens. The
//! answer cannot reach 1.0 unless it holds every ground truth token
//! and no extra token. The union term is the recall term: it charges
//! the miner for every ground truth token it left out, and it charges
//! for every extra token it added.
//!
//! ## Why there is a token cap
//!
//! The overlap runs as a nested loop over both token lists, so the
//! work grows with the PRODUCT of the two token counts. The input cap
//! is 1 MiB, which holds about 500000 tokens, and a nested loop over
//! that many tokens would stop the validator. So this module reads at
//! most `MAX_TOKENS` tokens from each side and drops the rest.
//!
//! The cap is not a consensus risk. Every validator runs the SAME
//! registered binary, so every validator uses this same value by
//! construction. There is no way for two validators to disagree about
//! it. The cap exists only to bound the quadratic work.

/// The largest number of tokens this module reads from one text.
///
/// A real answer holds one value, so this cap is far above any honest
/// answer. The cap bounds the work of the nested overlap loop, which
/// grows with the product of the two token counts. It is not a
/// consensus constant; see the module doc comment.
pub const MAX_TOKENS: usize = 64;

/// The largest number of bytes in one token.
///
/// A longer token is cut at this length. Two tokens that share their
/// first 32 bytes are the same token for this module.
pub const MAX_TOKEN_BYTES: usize = 32;

/// A word that flips the meaning of a text.
///
/// The list is short and closed on purpose. This module does not build
/// a natural language system. It only detects the common English forms
/// that turn a statement into its opposite. The apostrophe forms
/// appear without the apostrophe, because normalisation drops
/// punctuation before this list runs.
const NEGATION_MARKERS: [&str; 13] = [
    "not", "no", "never", "none", "cannot", "cant", "isnt", "arent", "wasnt", "werent", "doesnt",
    "dont", "didnt",
];

/// This function tells if a character is an ASCII digit.
pub fn is_ascii_digit(character: char) -> bool {
    character.is_ascii_digit()
}

/// This function lowers one character, for ASCII letters only.
///
/// The function leaves every other character as it is. A full Unicode
/// case fold changes with the Unicode version, and a validator with a
/// different Unicode table would then compute a different score. An
/// ASCII fold is fixed for ever, so every validator agrees.
pub fn to_ascii_lowercase_char(character: char) -> char {
    if character.is_ascii_uppercase() {
        ((character as u8) + 32) as char
    } else {
        character
    }
}

/// This function tells if a character separates tokens.
///
/// A character is a separator when it is whitespace or punctuation.
/// A digit, a letter, and any other character stay inside a token.
fn is_separator(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '.' | ','
                | ';'
                | ':'
                | '!'
                | '?'
                | '"'
                | '\''
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '/'
                | '\\'
                | '|'
                | '_'
                | '*'
                | '#'
                | '@'
                | '`'
                | '~'
                | '<'
                | '>'
                | '='
                | '+'
                | '&'
                | '^'
        )
}

/// One token, held in a fixed byte buffer.
#[derive(Clone, Copy)]
pub struct Token {
    bytes: [u8; MAX_TOKEN_BYTES],
    length: usize,
}

impl Token {
    /// This function gives the token text.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length]).unwrap_or("")
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.length == other.length && self.bytes[..self.length] == other.bytes[..other.length]
    }
}

/// A set of tokens read from one text.
pub struct TokenSet {
    tokens: [Token; MAX_TOKENS],
    count: usize,
    /// True when the text held more tokens than the cap allows.
    pub truncated: bool,
    /// The number of negation markers in the text, before this module
    /// removed them from the token list.
    pub negation_count: usize,
}

impl TokenSet {
    /// This function gives the number of distinct tokens in the set.
    pub fn len(&self) -> usize {
        self.count
    }

    /// This function tells if the set holds no token.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// This function gives the tokens as a slice.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens[..self.count]
    }

    /// This function tells if the set holds a token.
    pub fn contains(&self, needle: &Token) -> bool {
        self.tokens().iter().any(|token| token == needle)
    }
}

/// This function normalises a text and builds its token set.
///
/// The function lowers ASCII letters, splits on whitespace and
/// punctuation, drops empty parts, and keeps each distinct token one
/// time. It counts and then removes the negation markers, so that the
/// caller can compare the meaning and the negation apart from each
/// other.
pub fn tokenize(text: &str) -> TokenSet {
    let empty = Token {
        bytes: [0u8; MAX_TOKEN_BYTES],
        length: 0,
    };
    let mut set = TokenSet {
        tokens: [empty; MAX_TOKENS],
        count: 0,
        truncated: false,
        negation_count: 0,
    };

    let mut current = [0u8; MAX_TOKEN_BYTES];
    let mut current_length = 0usize;

    // A closure cannot borrow `set` and `current` at the same time
    // here, so the flush step is written out in full, two times.
    for character in text.chars() {
        if is_separator(character) {
            if current_length > 0 {
                push_token(&mut set, &current, current_length);
                current_length = 0;
            }
            continue;
        }
        let lowered = to_ascii_lowercase_char(character);
        let mut encode = [0u8; 4];
        let encoded = lowered.encode_utf8(&mut encode);
        if current_length + encoded.len() <= MAX_TOKEN_BYTES {
            current[current_length..current_length + encoded.len()]
                .copy_from_slice(encoded.as_bytes());
            current_length += encoded.len();
        }
        // A token longer than the cap keeps its first bytes only.
    }
    if current_length > 0 {
        push_token(&mut set, &current, current_length);
    }

    set
}

/// This function adds one token to a set, if the set has room.
///
/// The function counts a negation marker and does not store it. It
/// also drops a token that the set already holds, so the set stays a
/// set and not a list.
fn push_token(set: &mut TokenSet, bytes: &[u8; MAX_TOKEN_BYTES], length: usize) {
    let text = match core::str::from_utf8(&bytes[..length]) {
        Ok(value) => value,
        Err(_) => return,
    };

    for marker in NEGATION_MARKERS.iter() {
        if text == *marker {
            set.negation_count += 1;
            return;
        }
    }

    let candidate = Token {
        bytes: *bytes,
        length,
    };
    if set.contains(&candidate) {
        return;
    }
    if set.count >= MAX_TOKENS {
        set.truncated = true;
        return;
    }
    set.tokens[set.count] = candidate;
    set.count += 1;
}

/// This function counts the tokens that both sets hold.
pub fn intersection_size(left: &TokenSet, right: &TokenSet) -> usize {
    let mut shared = 0usize;
    for token in left.tokens() {
        if right.contains(token) {
            shared += 1;
        }
    }
    shared
}

/// This function scores the overlap of two token sets.
///
/// The function divides the shared token count by the union token
/// count. The result is 1.0 only when both sets hold exactly the same
/// tokens. The result is 0.0 when the sets share no token.
///
/// The union divisor is what stops the "answer with one common word"
/// attack. See the module doc comment.
pub fn overlap_score(ground_truth: &TokenSet, answer: &TokenSet) -> f64 {
    if ground_truth.is_empty() && answer.is_empty() {
        return 0.0;
    }
    let shared = intersection_size(ground_truth, answer);
    let union = ground_truth.len() + answer.len() - shared;
    if union == 0 {
        return 0.0;
    }
    // The counts are small, so this conversion is exact.
    (shared as f64) / (union as f64)
}

/// This function tells if two texts disagree about negation.
///
/// The function compares the parity of the negation marker counts. An
/// even count and an odd count disagree. Two negations cancel, so
/// "not not malicious" agrees with "malicious".
pub fn negation_disagrees(ground_truth: &TokenSet, answer: &TokenSet) -> bool {
    (ground_truth.negation_count % 2) != (answer.negation_count % 2)
}
