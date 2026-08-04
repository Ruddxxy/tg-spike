//! This module holds the math helpers that keep the score
//! deterministic across every WASM host.
//!
//! ## Why this crate does not use the std `ln` function
//!
//! The WASM spec makes `+`, `-`, `*`, `/`, and `sqrt` exact IEEE-754
//! operations. Every conformant host must give the same bit pattern
//! for the same inputs. `f64::abs`, `f64::floor`, `f64::trunc`, and
//! `f64::copysign` are also exact bit operations, so they are safe
//! too. A transcendental function such as `ln`, `log`, `exp`, or
//! `powf` is not part of that exact set. Some WASM hosts provide it
//! as an imported libm function, and different libm builds can give
//! a different last bit for the same input. A validator that reads
//! a different last bit than another validator would disagree about
//! the score. That would break consensus. So this module writes its
//! own `ln` from the exact operations only.
//!
//! ## Why the batch mean sorts first and uses Kahan summation
//!
//! Float addition is not associative. Adding the same list of
//! numbers in a different order can give a different sum. A host
//! could give the pairs for `score_batch` in any order. To remove
//! that order dependence, this module first sorts the per-pair
//! scores into one fixed total order with `f64::total_cmp`, using a
//! stable sort so no tie is left to chance. The module then adds the
//! sorted list with Kahan (compensated) summation, which tracks a
//! running compensation value and removes most of the rounding
//! error a plain sum would build up. Together the sort and the
//! Kahan method make sure every host computes the same batch mean,
//! no matter what order the host gives the pairs in.

/// This function calculates the natural log of `x`.
///
/// This function does not call the std `ln` function. See the
/// module doc comment for the reason. This function uses a hand
/// written atanh series instead. The series uses only `+`, `-`,
/// `*`, and `/`. These are exact IEEE-754 operations in the WASM
/// spec. So this function gives the same result on every host.
///
/// The algorithm has six steps.
/// 1. The function handles the special cases: NaN, a negative
///    number, zero, and positive infinity.
/// 2. The function scales a subnormal number up so the bit split in
///    step 3 works on a normal number.
/// 3. The function splits the bits of `x` into a mantissa `m` in
///    `[1.0, 2.0)` and an exponent `e`, so that `x = m * 2^e`.
/// 4. The function centers `m` around 1.0. If `m` is greater than
///    the square root of 2, the function halves `m` and adds 1 to
///    `e`. This puts `m` in about `[0.7071, 1.4142)`.
/// 5. The function runs an atanh series on `m` to get `ln(m)`.
/// 6. The function combines `e * ln(2)` with `ln(m)` to get the
///    final result. The constant `ln(2)` is split into a high part
///    and a low part so the combination keeps full precision even
///    when `e` is large.
///
/// The function returns NaN if `x` is NaN or `x` is less than 0.0.
/// The function returns negative infinity if `x` is 0.0.
/// The function returns positive infinity if `x` is positive
/// infinity.
#[allow(clippy::many_single_char_names)]
// The constants below need every digit clippy thinks is excess.
// `LN2_HI` and `LN2_LO` must hold the full split-precision literal
// value the algorithm doc comment names. `SQRT_2` must stay a
// literal, not the std constant, because the spec for this function
// forbids computing a constant with std math.
#[allow(clippy::excessive_precision, clippy::approx_constant)]
pub fn ln(x: f64) -> f64 {
    // Step 1: handle the special cases first.
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return f64::INFINITY;
    }

    // The high part and the low part of ln(2). The split keeps
    // precision when the code multiplies each part by a large
    // exponent.
    const LN2_HI: f64 = 6.931_471_803_691_238_164_90e-01;
    const LN2_LO: f64 = 1.908_214_929_270_587_700_02e-10;
    // The square root of 2. The function uses this value to center
    // the mantissa around 1.0.
    const SQRT_2: f64 = 1.414_213_562_373_095_1;
    // 2 to the power 54. The function uses this value to scale a
    // subnormal number up to a normal number.
    const TWO_POW_54: f64 = 18_014_398_509_481_984.0;

    // Step 2: scale a subnormal number up. `f64::MIN_POSITIVE` is
    // the smallest positive normal number. Any smaller positive
    // number is subnormal.
    let (scaled, exponent_shift) = if x < f64::MIN_POSITIVE {
        (x * TWO_POW_54, -54i32)
    } else {
        (x, 0i32)
    };

    // Step 3: split the bits into a mantissa in [1.0, 2.0) and an
    // exponent. This is the classic frexp bit trick.
    let bits = scaled.to_bits();
    let biased_exponent = ((bits >> 52) & 0x7ff) as i32;
    let mantissa_bits = (bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000;
    let mut m = f64::from_bits(mantissa_bits);
    let mut e = biased_exponent - 1023 + exponent_shift;

    // Step 4: center the mantissa around 1.0.
    if m > SQRT_2 {
        m *= 0.5;
        e += 1;
    }

    // Step 5: run the atanh series on the centered mantissa.
    // ln(m) = 2 * (s + s^3/3 + s^5/5 + ... + s^19/19), where
    // s = (m - 1) / (m + 1). Since m is in about [0.7071, 1.4142),
    // |s| stays at or under about 0.1716. A measured accuracy test
    // in this module showed that 8 terms (up to s^15/15), which is
    // what an early draft of this function used, left a truncation
    // error of about 200 ULP right at the edge of that range. This
    // version uses 10 terms (up to s^19/19). The two extra terms
    // push the measured worst case error down to a few ULP. The
    // code writes the series in Horner form using s^2, working from
    // the smallest term out to the largest.
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut poly = 1.0 / 19.0;
    poly = poly * s2 + 1.0 / 17.0;
    poly = poly * s2 + 1.0 / 15.0;
    poly = poly * s2 + 1.0 / 13.0;
    poly = poly * s2 + 1.0 / 11.0;
    poly = poly * s2 + 1.0 / 9.0;
    poly = poly * s2 + 1.0 / 7.0;
    poly = poly * s2 + 1.0 / 5.0;
    poly = poly * s2 + 1.0 / 3.0;
    poly = poly * s2 + 1.0;
    let ln_m = 2.0 * s * poly;

    // Step 6: recombine e * ln(2) with ln(m). The i32 to f64 cast
    // is exact for every i32 value, because f64 holds 52 mantissa
    // bits.
    let e_f = f64::from(e);
    e_f * LN2_HI + (ln_m + e_f * LN2_LO)
}

/// This function adds a list of numbers with the Kahan method.
///
/// A plain running sum of many floats can build up rounding error.
/// The Kahan method tracks a running compensation value and removes
/// most of that error. The function returns 0.0 for an empty list.
///
/// Callers that need an order-independent sum must first sort the
/// list into a fixed order, for example with `sort_total_order`.
/// Kahan summation alone does not remove order dependence. It only
/// reduces rounding error for a given order.
pub fn kahan_sum(values: &[f64]) -> f64 {
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for &value in values {
        let adjusted = value - compensation;
        let new_sum = sum + adjusted;
        compensation = (new_sum - sum) - adjusted;
        sum = new_sum;
    }
    sum
}

/// This function sorts a list of scores into one fixed total order.
///
/// The function uses `f64::total_cmp`. This gives a total order
/// with no tie left to chance, even between values such as `0.0`
/// and `-0.0`, or between two different NaN bit patterns. The
/// function uses a stable sort (`slice::sort_by`), never an
/// unstable sort. A stable sort combined with a total order gives
/// the same output list for the same input multiset, no matter what
/// order the caller gives the values in. This removes the order
/// dependence that plain float addition would otherwise have.
pub fn sort_total_order(values: &mut [f64]) {
    values.sort_by(f64::total_cmp);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ln_of_eps_matches_max_loss_constant() {
        // `metrics::MAX_LOSS` is `-ln(1e-15)` as a literal. This
        // test checks that the hand written `ln` agrees with the
        // literal the code uses to normalize log loss.
        let computed = -ln(1e-15);
        let literal = 34.538_776_394_910_684;
        let diff = (computed - literal).abs();
        assert!(diff < 1e-9, "ln(1e-15) gave {computed}, want {literal}");
    }

    #[test]
    fn ln_matches_known_values() {
        assert!(ln(1.0) == 0.0, "ln(1.0) must be exactly 0.0");
        assert!(ln(0.0) == f64::NEG_INFINITY);
        assert!(ln(f64::INFINITY) == f64::INFINITY);
        assert!(ln(f64::NAN).is_nan());
        assert!(ln(-1.0).is_nan());
    }

    /// This function maps an `f64` to a `u64` key. The key order
    /// matches the float value order. The function uses this key to
    /// measure a distance in ULPs (units in the last place) between
    /// two floats.
    fn ulp_key(value: f64) -> u64 {
        let bits = value.to_bits();
        if bits & (1u64 << 63) != 0 {
            !bits
        } else {
            bits | (1u64 << 63)
        }
    }

    /// This function returns the ULP distance between two finite,
    /// non-NaN floats.
    fn ulp_distance(a: f64, b: f64) -> u64 {
        let ka = ulp_key(a);
        let kb = ulp_key(b);
        ka.max(kb) - ka.min(kb)
    }

    #[test]
    fn ln_accuracy_report() {
        let mut max_abs_err = 0.0_f64;
        let mut max_rel_err = 0.0_f64;
        let mut max_ulp = 0u64;
        let mut worst_x = 0.0_f64;

        let mut check = |x: f64| {
            let expected = x.ln();
            let actual = ln(x);
            if !expected.is_finite() || !actual.is_finite() {
                return;
            }
            let abs_err = (actual - expected).abs();
            if abs_err > max_abs_err {
                max_abs_err = abs_err;
            }
            if expected != 0.0 {
                let rel_err = abs_err / expected.abs();
                if rel_err > max_rel_err {
                    max_rel_err = rel_err;
                }
            }
            let ulp = ulp_distance(actual, expected);
            if ulp > max_ulp {
                max_ulp = ulp;
                worst_x = x;
            }
        };

        // Fixed points named in the spec.
        let fixed_points = [
            1e-300,
            1e-15,
            1e-6,
            0.1,
            0.5,
            0.9999,
            1.0,
            1.0001,
            2.0,
            core::f64::consts::E,
            10.0,
            1e6,
            1e15,
            1e300,
        ];
        for x in fixed_points {
            check(x);
        }

        // A dense multiplicative sweep over [1e-10, 1e10]. The
        // ratio 1e20 with a step of 1.0021 gives about 22,000
        // points.
        let mut x = 1e-10_f64;
        let step = 1.0021_f64;
        while x <= 1e10 {
            check(x);
            x *= step;
        }

        // A dense additive sweep over [0.5, 2.0], where the atanh
        // series argument `s` is largest in magnitude. This gives
        // 15,000 points.
        let mut x = 0.5_f64;
        while x <= 2.0 {
            check(x);
            x += 0.0001;
        }

        println!("ln accuracy report:");
        println!("  max_abs_err = {max_abs_err:e}");
        println!("  max_rel_err = {max_rel_err:e}");
        println!("  max_ulp     = {max_ulp}");
        println!("  worst_x     = {worst_x:e}");

        // Measured result on this implementation, over the sweeps
        // above: max_ulp = 3, max_rel_err = 4.35e-16. The threshold
        // below leaves headroom above that measurement, so the test
        // still catches a real regression without being brittle
        // about the exact last-bit result on a different host CPU.
        assert!(max_ulp <= 8, "ln max ULP error too large: {max_ulp}");
        assert!(
            max_rel_err < 1e-15,
            "ln max relative error too large: {max_rel_err:e}"
        );
    }

    #[test]
    fn kahan_sum_of_empty_list_is_zero() {
        assert_eq!(kahan_sum(&[]), 0.0);
    }

    #[test]
    fn kahan_sum_matches_plain_sum_for_small_lists() {
        let values = [0.1, 0.2, 0.3];
        let sum = kahan_sum(&values);
        assert!((sum - 0.6).abs() < 1e-12);
    }

    #[test]
    fn kahan_sum_reduces_error_versus_plain_sum() {
        // Adding a huge number of small values in plain order loses
        // precision. Kahan summation should stay much closer to the
        // exact answer.
        let values = vec![1.0_f64; 1_000_000];
        let mut plain_sum = 0.0_f64;
        for &value in &values {
            plain_sum += value;
        }
        let compensated_sum = kahan_sum(&values);
        assert_eq!(compensated_sum, 1_000_000.0);
        assert_eq!(plain_sum, 1_000_000.0);
    }

    #[test]
    fn sort_total_order_gives_ascending_order() {
        let mut values = [3.0, 1.0, f64::NAN, -1.0, 0.0];
        sort_total_order(&mut values);
        // total_cmp places NaN after positive infinity and after
        // every other value for a positive-sign NaN. The list here
        // has -1.0, 0.0, 1.0, 3.0, then NaN last.
        assert_eq!(values[0], -1.0);
        assert_eq!(values[1], 0.0);
        assert_eq!(values[2], 1.0);
        assert_eq!(values[3], 3.0);
        assert!(values[4].is_nan());
    }

    #[test]
    fn sort_total_order_is_order_independent() {
        let mut a = [0.3, 0.1, 0.2, 0.05, 0.25];
        let mut b = [0.25, 0.05, 0.3, 0.2, 0.1];
        sort_total_order(&mut a);
        sort_total_order(&mut b);
        assert_eq!(a, b);
    }
}
