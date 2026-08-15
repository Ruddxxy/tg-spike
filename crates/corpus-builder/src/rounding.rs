//! Single source of truth for temperature rounding.
//!
//! This crate has ONE rounding function, used by every ground-truth
//! rendering and by `actual_c`, so they can never disagree with each
//! other. All of that happens here.

/// Round a Celsius value to one decimal place.
///
/// This is the ONLY place in the crate that rounds a temperature.
/// `gt_bare`, `gt_prose`, `gt_json`, and `actual_c` all call this function
/// (directly or through [`format_temp_c`]).
///
/// Rounding rule: HALF AWAY FROM ZERO, not half-to-even ("banker's
/// rounding"). This is the rule `f64::round` implements. Example: 18.25
/// rounds to 18.3 (not 18.2), and -18.25 rounds to -18.3 (not -18.2).
pub fn round_temp_c(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Format a Celsius value to one decimal place, e.g. `"18.6"`.
///
/// Always rounds first with [`round_temp_c`], so this string and the
/// `temperature_2m` number written into `gt_json` always show the same
/// digit.
pub fn format_temp_c(value: f64) -> String {
    format!("{:.1}", round_temp_c(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_down_below_half() {
        assert_eq!(round_temp_c(18.64), 18.6);
    }

    #[test]
    fn rounds_up_above_half() {
        assert_eq!(round_temp_c(18.66), 18.7);
    }

    #[test]
    fn half_case_rounds_away_from_zero_positive() {
        // 18.25 * 10 = 182.5, f64::round on 182.5 goes to 183 (away from
        // zero), so this must land on 18.3, not 18.2.
        assert_eq!(round_temp_c(18.25), 18.3);
    }

    #[test]
    fn half_case_rounds_away_from_zero_negative() {
        assert_eq!(round_temp_c(-18.25), -18.3);
    }

    #[test]
    fn format_matches_bare_example_from_the_brief() {
        assert_eq!(format_temp_c(18.6), "18.6");
    }
}
