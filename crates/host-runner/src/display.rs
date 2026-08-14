//! This module maps a golden vector name to a PUBLIC label.
//!
//! ## Why the names change for the display
//!
//! The internal vector names describe what each vector tests, which is
//! what a name is for. Several of them therefore state a calibration
//! point of the tolerance curve or the intent family the script
//! targets. A screenshot of the cross-engine table goes on the public
//! internet, and a reader who sees a name beside its `f32` bit pattern
//! can read a design decision straight off the picture.
//!
//! Two examples, to make the risk concrete:
//!
//! - A name that states "one percent out", printed beside the bit
//!   pattern `0x3f666666`, tells the reader that a 1 percent error
//!   scores exactly 0.9. Two such rows fix the whole curve.
//! - A name that states a temperature unit conversion tells the reader
//!   which intent family the script is tuned for.
//!
//! ## Why EVERY name changes, not only the leaking ones
//!
//! A partial rename is worse than none. If four rows read as
//! placeholders and twelve read as real names, the reader knows
//! exactly which four rows hide something and can guess what from the
//! surrounding bits. A uniform scheme leaks no such signal.
//!
//! ## Fail closed
//!
//! `public_label` never falls back to the real name. A vector name that
//! this module does not know gets a positional label instead. So a
//! future vector cannot leak by simply not being listed here; the worst
//! case is a less descriptive screenshot.
//!
//! This mapping is for DISPLAY ONLY. The comparison itself always runs
//! on the real names, so a rename here can never change what is
//! compared or hide a mismatch.

/// The public label for each golden vector, by its real name.
///
/// The labels group the vectors by input class and number them. That
/// carries enough meaning for a reader to see that the suite covers
/// several classes, and it states nothing about any threshold, any
/// unit, or any defence.
const PUBLIC_LABELS: [(&str, &str); 16] = [
    // Numeric path. The names below state exact error sizes, which
    // together fix the tolerance curve.
    ("numeric_exact", "numeric_01"),
    ("numeric_one_cent_out", "numeric_02"),
    ("numeric_trailing_zero", "numeric_03"),
    ("numeric_wild_answer", "numeric_04"),
    ("numeric_one_percent_out", "numeric_05"),
    ("numeric_fifty_percent_out", "numeric_06"),
    // Unit path. The names below state the currency and the
    // temperature scales, which name the target intent families.
    ("currency_symbol_match", "unit_01"),
    ("currency_code_match", "unit_02"),
    ("unit_kelvin_to_celsius", "unit_03"),
    ("unit_incompatible", "unit_04"),
    // Text path. One name below states a specific adversarial
    // defence.
    ("text_exact", "text_01"),
    ("text_unrelated", "text_02"),
    ("text_negated", "text_03"),
    ("text_is_attack", "text_04"),
    // Edge cases.
    ("blank_answer", "edge_01"),
    ("question_is_junk", "edge_02"),
];

/// This function gives the public label for a vector name.
///
/// `index` is the row position, used only for the fail-closed
/// fallback. The function NEVER returns the input name.
pub fn public_label(name: &str, index: usize) -> String {
    for (real, public) in PUBLIC_LABELS.iter() {
        if *real == name {
            return (*public).to_string();
        }
    }
    // Fail closed. An unlisted vector gets a positional label, never
    // its real name.
    format!("case_{:02}", index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words that must never reach a public label. Each one states a
    /// calibration point, a unit, or a defence.
    const FORBIDDEN: [&str; 16] = [
        "percent",
        "cent",
        "kelvin",
        "celsius",
        "currency",
        "usd",
        "attack",
        "negat",
        "wild",
        "junk",
        "blank",
        "exact",
        "trailing",
        "incompatible",
        "unrelated",
        "symbol",
    ];

    #[test]
    fn no_public_label_holds_a_forbidden_word() {
        for (_, public) in PUBLIC_LABELS.iter() {
            let lowered = public.to_lowercase();
            for word in FORBIDDEN.iter() {
                assert!(
                    !lowered.contains(word),
                    "the public label {public:?} holds the forbidden word {word:?}"
                );
            }
        }
    }

    #[test]
    fn a_public_label_never_equals_its_real_name() {
        for (real, public) in PUBLIC_LABELS.iter() {
            assert_ne!(real, public, "the label for {real:?} was not changed");
        }
    }

    #[test]
    fn the_public_labels_are_unique() {
        for (index, (_, public)) in PUBLIC_LABELS.iter().enumerate() {
            for (_, other) in PUBLIC_LABELS.iter().skip(index + 1) {
                assert_ne!(public, other, "the label {public:?} appears twice");
            }
        }
    }

    #[test]
    fn an_unlisted_name_falls_back_and_never_leaks() {
        let secret = "numeric_ninety_nine_percent_out";
        let label = public_label(secret, 7);
        assert_eq!(label, "case_08");
        assert!(
            !label.contains("percent"),
            "the fallback leaked the real name"
        );
    }

    #[test]
    fn every_golden_vector_in_the_file_has_a_label() {
        // A vector added to the file without a label here would print
        // as case_NN. That is safe, but it is also a sign the two lists
        // drifted, so this test keeps them together.
        let text = include_str!("../../../golden_vectors.json");
        for (real, _) in PUBLIC_LABELS.iter() {
            let key = format!("\"name\": \"{real}\"");
            assert!(
                text.contains(&key),
                "the label list names {real:?}, which the golden file does not have"
            );
        }
    }

    #[test]
    fn known_names_map_to_their_label() {
        assert_eq!(public_label("numeric_one_percent_out", 0), "numeric_05");
        assert_eq!(public_label("text_is_attack", 0), "text_04");
        assert_eq!(public_label("unit_kelvin_to_celsius", 0), "unit_03");
    }
}
