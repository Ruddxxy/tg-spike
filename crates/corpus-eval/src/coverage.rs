//! This module measures the value parser against real corpus text.
//!
//! The unit tests exercise the parser against hand-written text only.
//! This module runs it over the real strings in the corpus and reports
//! what it refuses.
//!
//! ## Three different questions
//!
//! The module measures three input sets, because they answer three
//! different questions and only one of them is about the shipped ABI:
//!
//! 1. The EXTRACTED miner value. This is what `rank_answer` really
//!    receives, because the protocol standardises a miner answer into
//!    a single value before the module sees it. A failure here is a
//!    real defect.
//! 2. The GROUND TRUTH renderings. A failure here is also a real
//!    defect, because the module must read whatever the protocol
//!    sends.
//! 3. The RAW upstream miner response. This is the full JSON blob the
//!    corpus stores. `rank_answer` never sees this. The number is
//!    reported anyway, because it shows what the parser does with
//!    text far outside its shape, and because a reader will ask.

use std::collections::BTreeMap;

use eval_script::value::{parse_value, scan_values, ParsedValue, Unit, MAX_SCANNED_VALUES};

/// One coverage result for one input set.
pub struct CoverageReport {
    /// The name of the input set.
    pub name: String,
    /// How many texts were tested.
    pub total: usize,
    /// How many gave a value from the STRICT parser.
    pub strict_ok: usize,
    /// How many gave at least one value from the LENIENT scanner.
    pub scan_ok: usize,
    /// Shapes that the lenient scanner found nothing in, counted.
    pub unscanned_shapes: BTreeMap<String, (usize, String)>,
}

impl CoverageReport {
    /// This function gives the strict pass rate as a percentage.
    pub fn strict_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        100.0 * (self.strict_ok as f64) / (self.total as f64)
    }

    /// This function gives the lenient pass rate as a percentage.
    pub fn scan_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        100.0 * (self.scan_ok as f64) / (self.total as f64)
    }
}

/// This function turns a text into a shape signature.
///
/// The signature replaces every digit with `9` and every run of
/// letters with `A`, so that many texts that differ only in their
/// values collapse into one shape. This is what makes a "top 20
/// shapes" list readable instead of a list of 6000 unique strings.
pub fn shape_of(text: &str) -> String {
    let mut shape = String::new();
    let mut last_was_letter = false;
    for character in text.chars().take(400) {
        if character.is_ascii_digit() {
            shape.push('9');
            last_was_letter = false;
        } else if character.is_alphabetic() {
            if !last_was_letter {
                shape.push('A');
            }
            last_was_letter = true;
        } else {
            shape.push(character);
            last_was_letter = false;
        }
        if shape.len() >= 70 {
            shape.push_str("...");
            break;
        }
    }
    shape
}

/// This function measures one input set.
pub fn measure<'a>(name: &str, texts: impl Iterator<Item = &'a str>) -> CoverageReport {
    let mut report = CoverageReport {
        name: name.to_string(),
        total: 0,
        strict_ok: 0,
        scan_ok: 0,
        unscanned_shapes: BTreeMap::new(),
    };

    let mut buffer = [ParsedValue {
        number: 0.0,
        unit: Unit::None,
    }; MAX_SCANNED_VALUES];

    for text in texts {
        report.total += 1;
        if parse_value(text).is_some() {
            report.strict_ok += 1;
        }
        let found = scan_values(text, &mut buffer);
        if found > 0 {
            report.scan_ok += 1;
        } else {
            let shape = shape_of(text);
            let entry = report
                .unscanned_shapes
                .entry(shape)
                .or_insert((0, text.chars().take(90).collect()));
            entry.0 += 1;
        }
    }

    report
}

/// This function prints one coverage report.
pub fn print_report(report: &CoverageReport, top: usize) {
    println!("--- {} ---", report.name);
    println!("texts tested:                     {}", report.total);
    println!(
        "strict parse_value gave a value:  {} ({:.2}%)",
        report.strict_ok,
        report.strict_rate()
    );
    println!(
        "lenient scan found a quantity:    {} ({:.2}%)",
        report.scan_ok,
        report.scan_rate()
    );
    let unscanned: usize = report.total - report.scan_ok;
    println!(
        "no quantity found at all:         {} ({:.2}%)",
        unscanned,
        100.0 * (unscanned as f64) / (report.total.max(1) as f64)
    );

    if report.unscanned_shapes.is_empty() {
        println!("every text yielded a quantity.");
        println!();
        return;
    }

    let mut shapes: Vec<(&String, &(usize, String))> = report.unscanned_shapes.iter().collect();
    shapes.sort_by(|left, right| right.1 .0.cmp(&left.1 .0).then_with(|| left.0.cmp(right.0)));

    println!();
    println!("top {top} shapes with NO quantity found:");
    println!("{:>7}  {:<74}  example", "count", "shape");
    for (shape, (count, example)) in shapes.iter().take(top) {
        println!("{count:>7}  {shape:<74}  {example:?}");
    }
    println!("distinct shapes with no quantity: {}", shapes.len());
    println!();
}
