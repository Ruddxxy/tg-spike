//! This test file covers every value the parser must read, and every
//! value it must refuse.
//!
//! The table in the wave brief drives these tests. Each case in the
//! brief has a test here.

use eval_script::value::{parse_value, Family, Unit};

/// This helper checks that a text gives a number and a unit.
fn expect(text: &str, number: f64, unit: Unit) {
    let parsed = parse_value(text)
        .unwrap_or_else(|| panic!("{text:?} must parse, but the parser returned None"));
    assert!(
        (parsed.number - number).abs() < 1e-9,
        "{text:?} gave number {}, want {number}",
        parsed.number
    );
    assert_eq!(parsed.unit, unit, "{text:?} gave the wrong unit");
}

/// This helper checks that a text gives nothing.
fn expect_none(text: &str) {
    let parsed = parse_value(text);
    assert!(
        parsed.is_none(),
        "{text:?} must give None, but it gave {parsed:?}"
    );
}

#[test]
fn reads_plain_numbers() {
    expect("42", 42.0, Unit::None);
    expect("-3.5", -3.5, Unit::None);
    expect("0", 0.0, Unit::None);
}

#[test]
fn reads_us_group_separators() {
    expect("1,234.56", 1234.56, Unit::None);
}

#[test]
fn reads_eu_group_separators() {
    expect("1.234,56", 1234.56, Unit::None);
}

#[test]
fn reads_space_group_separators() {
    expect("1 234,56", 1234.56, Unit::None);
}

#[test]
fn reads_a_lone_comma_as_a_decimal_mark() {
    // A European feed writes 192.43 as "192,43". Two digits after the
    // comma cannot be a group of three, so this reading is the only
    // one.
    expect("192,43", 192.43, Unit::None);
}

#[test]
fn refuses_the_ambiguous_thousands_comma() {
    // "1,234" is 1234 in the United States and 1.234 in Germany. The
    // two readings differ by a factor of 1000. The parser must not
    // pick one.
    expect_none("1,234");
}

#[test]
fn reads_repeated_group_commas() {
    // More than one comma, each with three digits after it, can only
    // be a group mark.
    expect("12,345,678", 12345678.0, Unit::None);
}

#[test]
fn reads_currency_symbols_and_codes() {
    expect("$192.43", 192.43, Unit::Usd);
    expect("192.43 USD", 192.43, Unit::Usd);
    expect("USD 192.43", 192.43, Unit::Usd);
    expect("\u{20b9}1200", 1200.0, Unit::Inr);
}

#[test]
fn reads_temperature_units() {
    expect("34.7 C", 34.7, Unit::Celsius);
    expect("34.7\u{b0}C", 34.7, Unit::Celsius);
    expect("307.85 K", 307.85, Unit::Kelvin);
    expect("94.4 F", 94.4, Unit::Fahrenheit);
}

#[test]
fn reads_percent_and_gas_units() {
    expect("15%", 15.0, Unit::Percent);
    expect("15 %", 15.0, Unit::Percent);
    expect("12 gwei", 12.0, Unit::Gwei);
}

#[test]
fn reads_scientific_notation() {
    expect("1.2e6", 1_200_000.0, Unit::None);
    expect("1.2E-3", 0.0012, Unit::None);
}

#[test]
fn reads_every_negative_form() {
    expect("-5", -5.0, Unit::None);
    // The accounting form wraps a negative number in brackets.
    expect("(5)", -5.0, Unit::None);
    // U+2212 is the real minus sign. A data feed may send it.
    expect("\u{2212}5", -5.0, Unit::None);
}

#[test]
fn reads_through_noise() {
    expect("  42  ", 42.0, Unit::None);
    expect("42.", 42.0, Unit::None);
    expect("about 42", 42.0, Unit::None);
}

#[test]
fn refuses_texts_that_are_not_numbers() {
    expect_none("sunny");
    expect_none("malicious");
    expect_none("");
    expect_none("   ");
    expect_none("N/A");
    expect_none("null");
}

#[test]
fn refuses_a_text_with_two_numbers() {
    // A range is not one value. The scorer sends this to the text
    // path, where the many-numbers penalty applies.
    expect_none("34 to 36");
}

#[test]
fn refuses_a_text_with_two_units() {
    expect_none("34.7 C 90 F");
}

#[test]
fn refuses_a_real_word_beside_a_number() {
    // "sunny 42" is not a clean value. The parser must not read it as
    // 42 and drop the word, because the word may carry the meaning.
    expect_none("sunny 42");
}

#[test]
fn refuses_a_non_finite_exponent() {
    // A value outside the f64 range must never reach the score.
    expect_none("1e400");
    expect_none("-1e400");
}

#[test]
fn space_grouping_does_not_join_two_separate_numbers() {
    // "34 7" is two numbers, not the grouped number 347, because "7"
    // is not a group of three digits.
    expect_none("34 7");
}

#[test]
fn temperature_converts_to_celsius() {
    let kelvin = parse_value("307.85 K").expect("kelvin must parse");
    assert!(
        (kelvin.to_base() - 34.7).abs() < 1e-9,
        "307.85 K gave {} C, want 34.7 C",
        kelvin.to_base()
    );
    let fahrenheit = parse_value("94.46 F").expect("fahrenheit must parse");
    assert!(
        (fahrenheit.to_base() - 34.7).abs() < 1e-6,
        "94.46 F gave {} C, want about 34.7 C",
        fahrenheit.to_base()
    );
}

#[test]
fn wei_converts_to_gwei() {
    let wei = parse_value("12000000000 wei").expect("wei must parse");
    assert!(
        (wei.to_base() - 12.0).abs() < 1e-9,
        "12000000000 wei gave {} gwei, want 12",
        wei.to_base()
    );
}

#[test]
fn each_currency_is_its_own_family() {
    // This crate never converts between currencies. A rate changes
    // with time, and this module has no clock and no market data.
    let dollars = parse_value("100 USD").expect("usd must parse");
    let euros = parse_value("100 EUR").expect("eur must parse");
    assert_ne!(dollars.family(), euros.family());
    assert_eq!(dollars.family(), Family::Usd);
    assert_eq!(euros.family(), Family::Eur);
}
