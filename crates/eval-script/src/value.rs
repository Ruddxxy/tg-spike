//! This module turns a short text into an optional number and unit.
//!
//! The Telegraph protocol team states that a miner answer is a single
//! extracted value, not a JSON document. The ground truth is a single
//! value too. The rendering of either side is not specified. So this
//! module must read many renderings of one number and must refuse to
//! guess when a text has more than one meaning.
//!
//! ## Why this module does not use a JSON parser
//!
//! An earlier version parsed both sides as JSON objects with a `label`
//! field and a `confidence` field. That model is gone. Both sides are
//! now short texts that hold one value each.
//!
//! ## How this module turns text into a number
//!
//! The module builds a clean ASCII form of the number in a small stack
//! buffer, then calls the `f64` parser in `core`. That parser is pure
//! Rust. It does not call a host function, and it rounds correctly. A
//! hand written digit loop would not give the same rounding on every
//! input, and a locale aware parser would not give the same result on
//! every host. Both would break consensus.
//!
//! ## The ambiguity rule
//!
//! A text with more than one meaning gives `None`. The module never
//! picks the more likely meaning. A wrong guess gives a wrong score,
//! and a wrong score on one validator is a slashing event. See
//! `parse_value` for the list of texts this module refuses.

use crate::text::{is_ascii_digit, to_ascii_lowercase_char};

/// The largest number of bytes in one numeric text.
///
/// A longer numeric text cannot change an `f64` result, because an
/// `f64` holds about 17 significant decimal digits. The cap stops a
/// very long digit run from filling the stack buffer.
const NUMERIC_BUFFER_BYTES: usize = 64;

/// The unit that a value carries.
///
/// The set is closed on purpose. A unit outside this set makes
/// `parse_value` return `None` for the unit part, not a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// No unit. The text holds a bare number.
    None,
    /// Degrees Celsius.
    Celsius,
    /// Degrees Fahrenheit.
    Fahrenheit,
    /// Kelvin.
    Kelvin,
    /// A percent value.
    Percent,
    /// United States dollars.
    Usd,
    /// Euros.
    Eur,
    /// Pounds sterling.
    Gbp,
    /// Indian rupees.
    Inr,
    /// Japanese yen.
    Jpy,
    /// Wei, the smallest Ethereum unit.
    Wei,
    /// Gwei. One gwei is 1e9 wei.
    Gwei,
}

/// The family of quantity that a unit measures.
///
/// Two values compare only when they have the same family. A
/// temperature and a price are different quantities. They are not
/// near each other at any distance.
///
/// Each currency is its own family. This module does NOT convert
/// between currencies. A rate between two currencies changes with
/// time, and this module has no clock and no market data. A converted
/// price would also differ between validators that used a different
/// rate, and that difference is a slashing event. So a dollar value
/// and a euro value are incomparable, and they score 0.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// A bare number with no unit.
    Dimensionless,
    /// A temperature. The module converts every temperature to Celsius.
    Temperature,
    /// A percent value.
    Percent,
    /// United States dollars.
    Usd,
    /// Euros.
    Eur,
    /// Pounds sterling.
    Gbp,
    /// Indian rupees.
    Inr,
    /// Japanese yen.
    Jpy,
    /// An Ethereum gas amount. The module converts every gas amount to
    /// gwei.
    Gas,
}

impl Unit {
    /// This function gives the quantity family of this unit.
    pub fn family(self) -> Family {
        match self {
            Unit::None => Family::Dimensionless,
            Unit::Celsius | Unit::Fahrenheit | Unit::Kelvin => Family::Temperature,
            Unit::Percent => Family::Percent,
            Unit::Usd => Family::Usd,
            Unit::Eur => Family::Eur,
            Unit::Gbp => Family::Gbp,
            Unit::Inr => Family::Inr,
            Unit::Jpy => Family::Jpy,
            Unit::Wei | Unit::Gwei => Family::Gas,
        }
    }
}

/// One value that `parse_value` read out of a text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedValue {
    /// The number, in the unit that `unit` names.
    pub number: f64,
    /// The unit of `number`.
    pub unit: Unit,
}

impl ParsedValue {
    /// This function gives the value in the base unit of its family.
    ///
    /// The base unit of a temperature is Celsius. The base unit of a
    /// gas amount is gwei. Every other family has one unit only, so
    /// the number does not change.
    ///
    /// The conversion runs with `+`, `-`, `*` and `/` only. Each of
    /// those is an exact IEEE-754 operation with one rounding step, so
    /// every host gives the same bits. A conversion through a
    /// transcendental function would not give that promise.
    pub fn to_base(self) -> f64 {
        match self.unit {
            Unit::Fahrenheit => (self.number - 32.0) * 5.0 / 9.0,
            Unit::Kelvin => self.number - 273.15,
            Unit::Wei => self.number / 1_000_000_000.0,
            _ => self.number,
        }
    }

    /// This function gives the quantity family of this value.
    pub fn family(self) -> Family {
        self.unit.family()
    }
}

/// A word that carries no meaning in front of a number.
///
/// A hedge word like "about" does not change the number that follows
/// it. The list is short and closed. This module does not try to read
/// natural language.
const NOISE_WORDS: [&str; 8] = [
    "about",
    "approx",
    "approximately",
    "around",
    "circa",
    "roughly",
    "nearly",
    "almost",
];

/// A text that names a missing value.
///
/// Each of these gives `None`, the same as any other text that is not
/// a number. The list exists so that the reason is clear in the code.
const NULL_WORDS: [&str; 6] = ["n/a", "na", "null", "none", "nil", "-"];

/// This function reads a unit name and gives the matching `Unit`.
///
/// The function returns `None` for a word outside the closed set.
fn unit_from_word(word: &str) -> Option<Unit> {
    // The match runs on an already lowercased word.
    match word {
        "c" | "celsius" => Some(Unit::Celsius),
        "f" | "fahrenheit" => Some(Unit::Fahrenheit),
        "k" | "kelvin" => Some(Unit::Kelvin),
        "%" | "percent" | "pct" => Some(Unit::Percent),
        "usd" | "dollar" | "dollars" => Some(Unit::Usd),
        "eur" | "euro" | "euros" => Some(Unit::Eur),
        "gbp" | "pound" | "pounds" => Some(Unit::Gbp),
        "inr" | "rupee" | "rupees" => Some(Unit::Inr),
        "jpy" | "yen" => Some(Unit::Jpy),
        "wei" => Some(Unit::Wei),
        "gwei" => Some(Unit::Gwei),
        _ => None,
    }
}

/// This function reads a currency symbol and gives the matching `Unit`.
fn unit_from_symbol(symbol: char) -> Option<Unit> {
    match symbol {
        '$' => Some(Unit::Usd),
        '\u{20ac}' => Some(Unit::Eur),
        '\u{a3}' => Some(Unit::Gbp),
        '\u{20b9}' => Some(Unit::Inr),
        '\u{a5}' => Some(Unit::Jpy),
        '%' => Some(Unit::Percent),
        '\u{b0}' => None,
        _ => None,
    }
}

/// This function tells if a character can start or continue a number.
fn is_numeric_char(character: char) -> bool {
    is_ascii_digit(character) || character == '.' || character == ','
}

/// This function tells if a character is a minus sign.
///
/// The set holds the ASCII hyphen and the Unicode minus sign U+2212.
/// A miner or a data feed may send either one.
fn is_minus(character: char) -> bool {
    character == '-' || character == '\u{2212}'
}

/// The result of a scan for one numeric run inside a text.
#[derive(Clone, Copy)]
struct NumericRun {
    /// The digits and separators of the run, with no sign.
    start: usize,
    /// One past the last byte of the run.
    end: usize,
}

/// This function finds every numeric run in a text.
///
/// A run is a maximal sequence of digits, dots and commas that holds
/// at least one digit. The function writes the runs into `out` and
/// gives the number found. A count above `out.len()` means the text
/// has more runs than the caller allows.
fn find_numeric_runs(text: &str, out: &mut [NumericRun]) -> usize {
    let mut count = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let character = text[index..].chars().next().unwrap_or(' ');
        let char_len = character.len_utf8();
        if is_numeric_char(character) {
            let start = index;
            let mut end = index;
            let mut has_digit = false;
            let mut cursor = index;
            while cursor < bytes.len() {
                let next = text[cursor..].chars().next().unwrap_or(' ');
                if !is_numeric_char(next) {
                    break;
                }
                if is_ascii_digit(next) {
                    has_digit = true;
                }
                cursor += next.len_utf8();
                end = cursor;
            }
            if has_digit {
                // Trim a trailing separator off the run.
                //
                // A run grows through dots and commas, so a number
                // inside JSON such as `:28.9,"time"` gives the run
                // "28.9," with the JSON comma on the end. The separator
                // rules would then see one dot AND one comma, decide
                // the LAST one is the decimal mark, and read "28.9," as
                // 289. A correct answer of 28.9 then scored 0.001
                // against a JSON ground truth.
                //
                // A trailing separator carries no digit, so trimming it
                // cannot change a real value. "42." still reads as 42.
                let mut trimmed_end = end;
                while trimmed_end > start {
                    let last = text[..trimmed_end].chars().next_back().unwrap_or('0');
                    if last == '.' || last == ',' {
                        trimmed_end -= last.len_utf8();
                    } else {
                        break;
                    }
                }
                if count < out.len() {
                    out[count] = NumericRun {
                        start,
                        end: trimmed_end,
                    };
                }
                count += 1;
            }
            index = if cursor > index {
                cursor
            } else {
                index + char_len
            };
        } else {
            index += char_len;
        }
    }
    count
}

/// This function decides how a run uses dots and commas, then writes
/// a clean ASCII number into `buffer`.
///
/// The function returns the number of bytes written, or `None` when
/// the run has more than one meaning or does not fit the buffer.
///
/// ## The separator rules
///
/// - Both a dot and a comma are present: the LAST one is the decimal
///   mark and the other one groups the digits. So "1,234.56" and
///   "1.234,56" both give 1234.56.
/// - Only a comma is present, and it has exactly three digits after
///   it, and it is the only comma: the text has two meanings. "1,234"
///   is 1234 with a group mark, and it is also 1.234 with a European
///   decimal mark. The function gives `None`.
/// - Only a comma is present, and every comma has exactly three
///   digits after it, and there is more than one comma: the commas
///   group the digits. So "12,345,678" gives 12345678.
/// - Only a comma is present in any other shape: the comma is a
///   decimal mark. So "192,43" gives 192.43.
/// - Only a dot is present: the dot is a decimal mark. So "192.430"
///   gives 192.43. This module does NOT treat a lone dot as a group
///   mark. A dot is the decimal mark in the protocol documents, and a
///   value such as "192.430" must keep its meaning.
fn write_clean_number(run: &str, buffer: &mut [u8]) -> Option<usize> {
    let dot_count = run.chars().filter(|c| *c == '.').count();
    let comma_count = run.chars().filter(|c| *c == ',').count();

    let last_dot = run.rfind('.');
    let last_comma = run.rfind(',');

    // Decide which character marks the decimal place.
    let decimal_mark: Option<char> = if dot_count > 0 && comma_count > 0 {
        match (last_dot, last_comma) {
            (Some(dot_at), Some(comma_at)) => {
                if dot_at > comma_at {
                    Some('.')
                } else {
                    Some(',')
                }
            }
            _ => None,
        }
    } else if comma_count > 0 {
        let digits_after_last_comma = match last_comma {
            Some(at) => run[at + 1..].chars().filter(|c| is_ascii_digit(*c)).count(),
            None => 0,
        };
        if comma_count == 1 && digits_after_last_comma == 3 {
            // "1,234" has two meanings. Refuse it.
            return None;
        }
        if comma_count > 1 {
            // Every comma must group exactly three digits, or the text
            // has no clear meaning.
            if !commas_group_correctly(run) {
                return None;
            }
            None
        } else {
            Some(',')
        }
    } else if dot_count > 1 {
        // "1.234.567" is a group form. Every dot must group three
        // digits, or the text has no clear meaning.
        if !dots_group_correctly(run) {
            return None;
        }
        None
    } else if dot_count == 1 {
        Some('.')
    } else {
        None
    };

    let mut written = 0usize;
    for character in run.chars() {
        if is_ascii_digit(character) {
            if written >= buffer.len() {
                return None;
            }
            buffer[written] = character as u8;
            written += 1;
        } else if Some(character) == decimal_mark {
            if written >= buffer.len() {
                return None;
            }
            buffer[written] = b'.';
            written += 1;
        }
        // Any other separator groups digits. Drop it.
    }
    if written == 0 {
        return None;
    }
    Some(written)
}

/// This function checks that every comma groups exactly three digits.
fn commas_group_correctly(run: &str) -> bool {
    let mut parts = run.split(',');
    let first = match parts.next() {
        Some(part) => part,
        None => return false,
    };
    if first.is_empty() || first.len() > 3 || !first.chars().all(is_ascii_digit) {
        return false;
    }
    for part in parts {
        if part.len() != 3 || !part.chars().all(is_ascii_digit) {
            return false;
        }
    }
    true
}

/// This function checks that every dot groups exactly three digits.
fn dots_group_correctly(run: &str) -> bool {
    let mut parts = run.split('.');
    let first = match parts.next() {
        Some(part) => part,
        None => return false,
    };
    if first.is_empty() || first.len() > 3 || !first.chars().all(is_ascii_digit) {
        return false;
    }
    for part in parts {
        if part.len() != 3 || !part.chars().all(is_ascii_digit) {
            return false;
        }
    }
    true
}

/// This function reads a text and gives one value, or `None`.
///
/// The function gives `None` when the text holds no number, when it
/// holds more than one number, when a separator makes the number
/// ambiguous, or when the text carries two different units.
///
/// ## Texts this function refuses, and the reason
///
/// - `""` and a whitespace only text: there is no number.
/// - `"sunny"`, `"malicious"`: there is no number.
/// - `"N/A"`, `"null"`, `"none"`, `"nil"`: these name a missing value.
/// - `"1,234"`: the comma is a group mark in one country and a decimal
///   mark in another. The two readings differ by a factor of 1000.
/// - `"34 to 36"`: the text holds two numbers. A range is not one
///   value. The caller handles a range through the text path instead.
/// - `"34.7 C 90 F"`: the text carries two units.
/// - A digit run longer than 64 bytes: it cannot change an `f64`.
///
/// ## The exponent form
///
/// The function reads `1.2e6` and `1.2E-3`. An exponent that gives a
/// value outside the `f64` range gives `None`, because a non-finite
/// number must never reach the score.
pub fn parse_value(text: &str) -> Option<ParsedValue> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // A text that names a missing value is not a number.
    let lowered_is_null = {
        let mut matches_any = false;
        for word in NULL_WORDS.iter() {
            if equals_ascii_lowercase(trimmed, word) {
                matches_any = true;
                break;
            }
        }
        matches_any
    };
    if lowered_is_null {
        return None;
    }

    // Read an exponent form first. The exponent letter sits between
    // digits, so the run scanner would split it into two runs.
    if let Some(value) = parse_exponent_form(trimmed) {
        return Some(value);
    }

    let mut runs = [
        NumericRun { start: 0, end: 0 },
        NumericRun { start: 0, end: 0 },
    ];
    let run_count = find_numeric_runs(trimmed, &mut runs);

    if run_count == 0 {
        return None;
    }
    if run_count > 1 {
        // A space can group digits, as in "1 234,56". Try that reading
        // before the function refuses the text.
        if let Some(value) = parse_space_grouped(trimmed) {
            return Some(value);
        }
        return None;
    }

    let run_text = &trimmed[runs[0].start..runs[0].end];
    let mut buffer = [0u8; NUMERIC_BUFFER_BYTES];
    let written = write_clean_number(run_text, &mut buffer)?;
    let clean = core::str::from_utf8(&buffer[..written]).ok()?;
    let magnitude: f64 = clean.parse().ok()?;
    if !magnitude.is_finite() {
        return None;
    }

    let before = &trimmed[..runs[0].start];
    let after = &trimmed[runs[0].end..];

    let negative = has_negative_marker(before, after);
    let unit = read_unit(before, after)?;

    let signed = if negative { -magnitude } else { magnitude };
    Some(ParsedValue {
        number: signed,
        unit,
    })
}

/// This function compares a text with a lowercase word, without a
/// heap allocation.
fn equals_ascii_lowercase(text: &str, lowercase_word: &str) -> bool {
    let mut text_chars = text.chars();
    let mut word_chars = lowercase_word.chars();
    loop {
        match (text_chars.next(), word_chars.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) => {
                if to_ascii_lowercase_char(a) != b {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// This function reads a number in exponent form, such as `1.2e6`.
///
/// The function gives `None` when the text is not in exponent form.
fn parse_exponent_form(text: &str) -> Option<ParsedValue> {
    // Find an `e` or `E` that has a digit before it and a digit or a
    // sign after it.
    let bytes = text.as_bytes();
    let mut marker_at: Option<usize> = None;
    // Whether any byte before the current one is a digit.
    //
    // The test used to read `bytes[..index].iter().any(is_ascii_digit)`,
    // which walks the whole text in front of EVERY `e`. A miner answer
    // of `e` repeated fills that with work that grows with the SQUARE of
    // the answer size, and the miner picks the size up to
    // `MAX_INPUT_BYTES`. At 128 KiB one call took 16 seconds under
    // wazero, and the cap is 1 MiB.
    //
    // "A digit appears before here" only ever turns from false to true
    // as the scan moves right, so one accumulator answers the same
    // question in one pass. The predicate is the same at every index,
    // so every score keeps the bits it had.
    let mut digit_seen = false;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'e' || *byte == b'E' {
            let has_digit_before = digit_seen;
            let rest = &bytes[index + 1..];
            let has_exponent_after = match rest.first() {
                Some(b'+') | Some(b'-') => rest.len() > 1 && rest[1].is_ascii_digit(),
                Some(byte_after) => byte_after.is_ascii_digit(),
                None => false,
            };
            if has_digit_before && has_exponent_after {
                if marker_at.is_some() {
                    // Two exponent markers. The text has no clear
                    // meaning.
                    return None;
                }
                marker_at = Some(index);
            }
        }
        if byte.is_ascii_digit() {
            digit_seen = true;
        }
    }
    let marker = marker_at?;

    let mantissa_text = &text[..marker];
    let exponent_text = &text[marker + 1..];

    // The exponent part must be a sign and digits only, with nothing
    // after it except an optional unit.
    let mut exponent_end = 0usize;
    let exponent_bytes = exponent_text.as_bytes();
    if matches!(exponent_bytes.first(), Some(b'+') | Some(b'-')) {
        exponent_end = 1;
    }
    while exponent_end < exponent_bytes.len() && exponent_bytes[exponent_end].is_ascii_digit() {
        exponent_end += 1;
    }
    let exponent_digits = &exponent_text[..exponent_end];
    let after_exponent = &exponent_text[exponent_end..];

    let mut mantissa_runs = [
        NumericRun { start: 0, end: 0 },
        NumericRun { start: 0, end: 0 },
    ];
    let mantissa_run_count = find_numeric_runs(mantissa_text, &mut mantissa_runs);
    if mantissa_run_count != 1 {
        return None;
    }
    let run_text = &mantissa_text[mantissa_runs[0].start..mantissa_runs[0].end];

    let mut buffer = [0u8; NUMERIC_BUFFER_BYTES];
    let written = write_clean_number(run_text, &mut buffer)?;

    // Join the mantissa and the exponent into one clean text.
    let mut joined = [0u8; NUMERIC_BUFFER_BYTES * 2];
    if written + 1 + exponent_digits.len() > joined.len() {
        return None;
    }
    joined[..written].copy_from_slice(&buffer[..written]);
    joined[written] = b'e';
    joined[written + 1..written + 1 + exponent_digits.len()]
        .copy_from_slice(exponent_digits.as_bytes());
    let total = written + 1 + exponent_digits.len();

    let clean = core::str::from_utf8(&joined[..total]).ok()?;
    let magnitude: f64 = clean.parse().ok()?;
    if !magnitude.is_finite() {
        return None;
    }

    let before = &mantissa_text[..mantissa_runs[0].start];
    let negative = has_negative_marker(before, "");
    let unit = read_unit(before, after_exponent)?;

    let signed = if negative { -magnitude } else { magnitude };
    Some(ParsedValue {
        number: signed,
        unit,
    })
}

/// This function reads a number whose digits are grouped by spaces,
/// such as "1 234,56".
///
/// A space groups digits only when the first group has one to three
/// digits and every later group has exactly three digits. So "1 234"
/// gives 1234, and "34 7" gives `None`, because "7" is not a group of
/// three. This rule stops the function from joining two separate
/// numbers into one.
fn parse_space_grouped(text: &str) -> Option<ParsedValue> {
    // Split off a leading sign and a unit, then look at the middle.
    let trimmed = text.trim();
    let mut first_digit = None;
    let mut last_numeric_end = 0usize;
    for (index, character) in trimmed.char_indices() {
        if is_ascii_digit(character) {
            if first_digit.is_none() {
                first_digit = Some(index);
            }
            last_numeric_end = index + character.len_utf8();
        }
    }
    let start = first_digit?;
    let middle = &trimmed[start..last_numeric_end];

    // The middle must hold digits, spaces, and at most one decimal
    // mark. Any other character means this is not one grouped number.
    let mut groups = [""; 8];
    let mut group_count = 0usize;
    for part in middle.split(' ') {
        if part.is_empty() {
            return None;
        }
        if group_count >= groups.len() {
            return None;
        }
        groups[group_count] = part;
        group_count += 1;
    }
    if group_count < 2 {
        return None;
    }

    // Every group except the first must be exactly three digits. The
    // last group may carry the decimal part.
    let mut buffer = [0u8; NUMERIC_BUFFER_BYTES];
    let mut written = 0usize;
    for (index, group) in groups.iter().take(group_count).enumerate() {
        let is_last = index + 1 == group_count;
        let digits_only = group.chars().all(is_ascii_digit);
        if index == 0 {
            if !digits_only || group.is_empty() || group.len() > 3 {
                return None;
            }
        } else if is_last {
            // The last group may hold a decimal mark.
            let leading_digits: usize = group.chars().take_while(|c| is_ascii_digit(*c)).count();
            if leading_digits != 3 {
                return None;
            }
        } else if !digits_only || group.len() != 3 {
            return None;
        }
        for character in group.chars() {
            if written >= buffer.len() {
                return None;
            }
            if is_ascii_digit(character) {
                buffer[written] = character as u8;
                written += 1;
            } else if character == ',' || character == '.' {
                buffer[written] = b'.';
                written += 1;
            } else {
                return None;
            }
        }
    }
    let clean = core::str::from_utf8(&buffer[..written]).ok()?;
    let magnitude: f64 = clean.parse().ok()?;
    if !magnitude.is_finite() {
        return None;
    }

    let before = &trimmed[..start];
    let after = &trimmed[last_numeric_end..];
    let negative = has_negative_marker(before, after);
    let unit = read_unit(before, after)?;
    let signed = if negative { -magnitude } else { magnitude };
    Some(ParsedValue {
        number: signed,
        unit,
    })
}

/// This function tells if the text around a number marks it negative.
///
/// The function reads a minus sign before the number, and it reads the
/// accounting form that wraps a number in brackets, as in "(5)".
fn has_negative_marker(before: &str, after: &str) -> bool {
    let mut minus_seen = false;
    for character in before.chars().rev() {
        if character.is_whitespace() {
            continue;
        }
        if is_minus(character) {
            minus_seen = true;
        }
        break;
    }
    let opens = before.chars().any(|c| c == '(');
    let closes = after.chars().any(|c| c == ')');
    minus_seen || (opens && closes)
}

/// This function reads the unit from the text around a number.
///
/// The function gives `Some(Unit::None)` when the text carries no
/// unit. It gives `None` when the text carries two different units,
/// or a word that is not a number, not a unit, and not a noise word.
/// A text such as "sunny 42" therefore gives `None`, so that the
/// caller sends it to the text path instead of scoring it as 42.
fn read_unit(before: &str, after: &str) -> Option<Unit> {
    let mut found: Option<Unit> = None;

    for section in [before, after] {
        for token in section.split(|c: char| c.is_whitespace()) {
            let cleaned = token.trim_matches(|c: char| {
                // A full stop is trimmed too. The numeric run scanner
                // now strips a trailing separator off the number, so a
                // text such as "42." leaves the "." here as context.
                // It is punctuation, never a unit.
                c == '('
                    || c == ')'
                    || c == ':'
                    || c == ';'
                    || c == ','
                    || c == '.'
                    || c == '"'
                    || c == '\''
            });
            if cleaned.is_empty() {
                continue;
            }
            // Read every character of the token. A token can hold a
            // symbol and a word together, as in "$192.43" where the
            // "before" text is "$".
            let mut word_start = 0usize;
            let mut saw_word_char = false;
            for (index, character) in cleaned.char_indices() {
                if let Some(symbol_unit) = unit_from_symbol(character) {
                    if !merge_unit(&mut found, symbol_unit) {
                        return None;
                    }
                    if !saw_word_char {
                        word_start = index + character.len_utf8();
                    }
                    continue;
                }
                if character == '\u{b0}' || is_minus(character) {
                    // A degree sign and a minus sign carry no unit.
                    if !saw_word_char {
                        word_start = index + character.len_utf8();
                    }
                    continue;
                }
                saw_word_char = true;
            }
            if word_start >= cleaned.len() {
                continue;
            }
            let word = &cleaned[word_start..];
            if word.is_empty() {
                continue;
            }
            if is_noise_word(word) {
                continue;
            }
            match lookup_unit_case_insensitive(word) {
                Some(unit) => {
                    if !merge_unit(&mut found, unit) {
                        return None;
                    }
                }
                None => {
                    // The token is a real word that is not a unit. The
                    // text is not a clean value.
                    return None;
                }
            }
        }
    }

    Some(found.unwrap_or(Unit::None))
}

/// This function records a unit, and reports a clash.
///
/// The function returns false when the text already carried a
/// different unit. Two different units in one text mean the text has
/// no single meaning.
fn merge_unit(found: &mut Option<Unit>, unit: Unit) -> bool {
    match found {
        Some(existing) => *existing == unit,
        None => {
            *found = Some(unit);
            true
        }
    }
}

/// This function tells if a word is a hedge word with no meaning.
fn is_noise_word(word: &str) -> bool {
    for noise in NOISE_WORDS.iter() {
        if equals_ascii_lowercase(word, noise) {
            return true;
        }
    }
    false
}

/// This function looks a unit word up without regard to letter case.
fn lookup_unit_case_insensitive(word: &str) -> Option<Unit> {
    let mut buffer = [0u8; 16];
    let mut written = 0usize;
    for character in word.chars() {
        let lowered = to_ascii_lowercase_char(character);
        let mut encode = [0u8; 4];
        let encoded = lowered.encode_utf8(&mut encode);
        if written + encoded.len() > buffer.len() {
            return None;
        }
        buffer[written..written + encoded.len()].copy_from_slice(encoded.as_bytes());
        written += encoded.len();
    }
    let lowered = core::str::from_utf8(&buffer[..written]).ok()?;
    unit_from_word(lowered)
}

/// The largest number of values that `scan_values` reports.
///
/// A ground truth in JSON form can hold a date, a coordinate, and the
/// value itself, so the scan needs room for more than a few. The cap
/// bounds the work.
pub const MAX_SCANNED_VALUES: usize = 32;

/// This function finds every quantity inside a text, leniently.
///
/// `parse_value` is STRICT: the whole text must be one clean value,
/// and any stray word makes it give `None`. This function is LENIENT:
/// it finds each numeric run anywhere inside the text and reads a unit
/// that sits next to it. It ignores every other character.
///
/// The two serve different questions. `parse_value` answers "is this
/// text one value?". This function answers "does this text CONTAIN a
/// value?". The scorer needs the second question for a ground truth
/// that arrives as prose or as JSON, where the value sits inside other
/// text.
///
/// The function writes into `out` and returns the number of values it
/// found, capped at `out.len()`.
///
/// A scan of JSON also picks up a date part and any digit inside a key
/// name. That is noise the caller must expect: the caller keeps the
/// BEST match, so extra numbers cost nothing unless one of them
/// happens to match. See `scan_truth_values` for the rule that removes
/// that noise from the GROUND TRUTH side.
pub fn scan_values(text: &str, out: &mut [ParsedValue]) -> usize {
    scan_runs_outside(text, &[], out)
}

/// The largest number of quoted strings this module inspects.
///
/// A ground truth in JSON form holds a handful of quoted strings. The
/// cap bounds the work. A ground truth with more quoted strings than
/// this keeps the digits in the later ones as candidates, which is the
/// behaviour this module had before the cap existed. That is safe:
/// the ground truth comes from the protocol's own pipeline, so a miner
/// cannot pad it to push a timestamp past the cap.
const MAX_QUOTED_SPANS: usize = 32;

/// A half open byte range inside a text.
#[derive(Clone, Copy)]
struct Span {
    /// The first byte of the range.
    start: usize,
    /// One past the last byte of the range.
    end: usize,
}

/// This function tells if a text is shaped like a JSON document.
///
/// The test is deliberately crude: the trimmed text opens with `{` or
/// `[` and closes with the matching bracket. This module has no JSON
/// parser and does not want one. The test only decides whether the
/// quoting rule in `scan_truth_values` applies, so a text that is not
/// JSON keeps exactly the behaviour it had before.
fn is_json_shaped(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 2 {
        return false;
    }
    let first = trimmed.chars().next();
    let last = trimmed.chars().next_back();
    matches!(
        (first, last),
        (Some('{'), Some('}')) | (Some('['), Some(']'))
    )
}

/// This function tells if a quoted string IS a value.
///
/// The test is `parse_value`, the module's existing strict reader, on
/// the whole string. Nothing else. A string it accepts is a quantity
/// and keeps its number as a match target; every other string is text.
///
/// # The property, and the weaker test that was here before
///
/// The property this rule needs is "the string IS a value". An earlier
/// version also accepted a string that CONTAINS one numeric run
/// standing on its own, so that a value wrapped in a sentence, as
/// `"28.9 C in Paris"` is, would keep its number.
///
/// Those are different properties, and the second one is a farm. It
/// admits every string that carries an incidental number:
///
/// | quoted string | admitted by the weak test | answer | score |
/// | --- | --- | --- | --- |
/// | `"HTTP 200"` | yes | `200` | 1.0000 |
/// | `"3 alerts active"` | yes | `3` | 1.0000 |
/// | `"revision 4"` | yes | `4` | 1.0000 |
/// | `"KJFK 12"` | yes | `12` | 1.0000 |
/// | `"Paris 2026"` | yes | `2026` | 1.0000 |
/// | `"6 hours"` | yes | `6` | 1.0000 |
///
/// Against a truth whose real value is a temperature, each of those
/// paid a WRONG answer a perfect score, while an honest miner 10
/// percent out earned 0.0831.
///
/// # Why there is no middle ground
///
/// `"28.9 C in Paris"` and `"200 OK"` have the same shape: a value,
/// then words. A rule that admits the first admits the second, and
/// `"3 alerts active"`, `"6 hours"` and `"2 of 3"` with it. No test on
/// the string's SYNTAX separates them, because the difference between
/// them is meaning. So the strict property is the only one that can be
/// stated, and the cost is real and is recorded: a JSON truth whose
/// only quantity sits inside a sentence in a quoted string now carries
/// no quantity, and falls to the text branch.
///
/// The rule never looks at a number's magnitude, so it cannot be aimed
/// at a chosen value.
///
/// # What it rejects, and what it keeps
///
/// Rejected: `"temperature_2m"`, a field name whose `2` is glued
/// inside a word. `"2026-08-10T12:00"`, which names an instant. Every
/// row of the table above.
///
/// Kept: `"28.9"`, `"28.9 C"`, `"28.9C"`, `" -5.2 "`, `"$192.43"`,
/// `"12 gwei"`, `"2026"` on its own. `parse_value` already handles the
/// surrounding whitespace and the unit suffix, so this function needs
/// no trimming and no unit list of its own.
fn quoted_string_reads_as_a_value(inner: &str) -> bool {
    parse_value(inner).is_some()
}

/// This function records every quoted string whose digits are TEXT.
///
/// A quoted string inside a JSON document is a key name or a string
/// value. Its digits belong to that text, not to a quantity: the `2`
/// in `"temperature_2m"` names a field, and the parts of
/// `"2026-08-10T12:00"` name an instant.
///
/// A quoted string that READS AS A VALUE is not recorded, so its number
/// stays a candidate. See `quoted_string_reads_as_a_value` for the two
/// ways a string can do that; the rule exists so that this function
/// never drops a legitimate quantity that happens to arrive quoted.
///
/// The function writes the recorded ranges into `out` and gives the
/// number found, which may exceed `out.len()`.
fn find_opaque_quoted_spans(text: &str, out: &mut [Span]) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut cursor = start;
        let mut end = bytes.len();
        while cursor < bytes.len() {
            let byte = bytes[cursor];
            if byte == b'\\' {
                // A backslash escapes the next byte, so a `\"` does not
                // close the string. A continuation byte of a multi byte
                // character is never `"` or `\`, so stepping over two
                // bytes here can never land the scan on a boundary that
                // matters.
                cursor += 2;
                continue;
            }
            if byte == b'"' {
                end = cursor;
                break;
            }
            cursor += 1;
        }
        // `get` gives `None` when a bound is not a character boundary.
        // Treat that as opaque: an unreadable string is not a value.
        let opaque = match text.get(start..end) {
            Some(inner) => !quoted_string_reads_as_a_value(inner),
            None => true,
        };
        if opaque {
            if count < out.len() {
                out[count] = Span { start, end };
            }
            count += 1;
        }
        index = if end < bytes.len() {
            end + 1
        } else {
            bytes.len()
        };
    }
    count
}

/// This function finds every quantity inside a GROUND TRUTH text.
///
/// It is `scan_values` with one extra rule for a JSON shaped text: a
/// number that sits inside a quoted string is text, not a quantity, so
/// it is not a candidate match target. A quoted string that reads as a
/// value keeps its number; see `quoted_string_reads_as_a_value`.
///
/// ## Why the ground truth needs this and the answer does not
///
/// The scorer keeps the BEST match over every (truth, answer) pair, so
/// every incidental number in the ground truth is a free target. With
/// the JSON rendering `{"temperature_2m":28.9,"time":"2026-08-10T12:00"}`
/// the scan used to yield 2, 28.9, 2026, -8, -10, 12 and 0, and an
/// answer of `2026`, `12`, `2` or `0` scored 1.0 while the same answer
/// scored under 0.001 against the bare and prose renderings of the same
/// truth. A registration check also failed on it: the unrelated answer
/// `12 gwei` matched the `12` of `T12:00` and tied the correct answer.
///
/// The rule is a fact about JSON, not a tuning constant: characters
/// between quotes are a string. It cannot be aimed at a chosen number,
/// because it never looks at any number's magnitude.
///
/// The ANSWER keeps the lenient scan. The answer's number count is the
/// anti-spray divisor, and dropping numbers from that count would make
/// a spray of quoted numbers cheaper than a spray of bare ones.
pub fn scan_truth_values(text: &str, out: &mut [ParsedValue]) -> usize {
    let mut spans = [Span { start: 0, end: 0 }; MAX_QUOTED_SPANS];
    let span_count = if is_json_shaped(text) {
        find_opaque_quoted_spans(text, &mut spans).min(spans.len())
    } else {
        0
    };
    scan_runs_outside(text, &spans[..span_count], out)
}

/// This function does the work of `scan_values`, skipping any run that
/// starts inside one of the given ranges.
fn scan_runs_outside(text: &str, skip: &[Span], out: &mut [ParsedValue]) -> usize {
    let mut runs = [NumericRun { start: 0, end: 0 }; MAX_SCANNED_VALUES];
    let run_count = find_numeric_runs(text, &mut runs).min(runs.len());

    let mut written = 0usize;
    for run in runs.iter().take(run_count) {
        if written >= out.len() {
            break;
        }
        let mut inside_quotes = false;
        for span in skip {
            if run.start >= span.start && run.start < span.end {
                inside_quotes = true;
                break;
            }
        }
        if inside_quotes {
            continue;
        }
        let run_text = &text[run.start..run.end];
        let mut buffer = [0u8; NUMERIC_BUFFER_BYTES];
        let cleaned = match write_clean_number(run_text, &mut buffer) {
            Some(length) => length,
            None => continue,
        };
        let clean = match core::str::from_utf8(&buffer[..cleaned]) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let magnitude: f64 = match clean.parse() {
            Ok(value) => value,
            Err(_) => continue,
        };
        if !magnitude.is_finite() {
            continue;
        }

        let before = &text[..run.start];
        let after = &text[run.end..];
        let unit = nearby_unit(before, after);
        let negative = leading_minus(before);
        let signed = if negative { -magnitude } else { magnitude };

        out[written] = ParsedValue {
            number: signed,
            unit,
        };
        written += 1;
    }
    written
}

/// This function reads a unit that sits next to a number.
///
/// The function looks at the character right before the number for a
/// currency symbol, and at the first word right after the number for a
/// unit name or a symbol. It ignores every other character, unlike
/// `read_unit`, which refuses a text that holds an unknown word.
fn nearby_unit(before: &str, after: &str) -> Unit {
    // A currency symbol sits right before the number, as in "$192.43".
    for character in before.chars().rev() {
        if character.is_whitespace() {
            continue;
        }
        if let Some(unit) = unit_from_symbol(character) {
            return unit;
        }
        break;
    }

    // A unit sits right after the number, as in "28.9 C" or "15%".
    let mut seen_gap = false;
    let mut word_start = None;
    let mut word_end = after.len();
    for (index, character) in after.char_indices() {
        if character.is_whitespace() || character == '\u{b0}' {
            if word_start.is_some() {
                word_end = index;
                break;
            }
            seen_gap = true;
            continue;
        }
        if let Some(unit) = unit_from_symbol(character) {
            if word_start.is_none() {
                return unit;
            }
        }
        if character.is_alphabetic() {
            if word_start.is_none() {
                word_start = Some(index);
            }
        } else {
            if word_start.is_some() {
                word_end = index;
            } else if seen_gap {
                // A non-letter after a gap ends the search.
                return Unit::None;
            } else {
                word_end = index;
            }
            break;
        }
    }
    if let Some(start) = word_start {
        let word = &after[start..word_end.max(start)];
        if let Some(unit) = lookup_unit_case_insensitive(word) {
            return unit;
        }
    }
    Unit::None
}

/// This function tells if a minus sign sits right before a number.
fn leading_minus(before: &str) -> bool {
    for character in before.chars().rev() {
        if character.is_whitespace() {
            continue;
        }
        return is_minus(character);
    }
    false
}
