//! This module gives a small deterministic PRNG.
//!
//! The generator is xorshift64*. The generator is not safe for use in
//! security code. The generator is fast and gives the same output for
//! the same seed on every machine, forever. That trait is the only
//! trait this crate needs.

/// A small deterministic PRNG.
///
/// The generator uses the xorshift64* algorithm. Two `Rng` values that
/// start from the same seed give the same sequence of outputs, in the
/// same order, forever.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

/// The state value that the generator uses when the seed is 0.
///
/// The xorshift64* algorithm gets stuck at state 0 forever: 0 xor any
/// shift of 0 is still 0. `new` swaps a seed of 0 for this constant so
/// the generator does not get stuck.
const ZERO_SEED_SUBSTITUTE: u64 = 0x9E37_79B9_7F4A_7C15;

/// The multiplier of the xorshift64* algorithm.
///
/// This constant comes from George Marsaglia's xorshift paper and the
/// xorshift64* variant. The value gives good output bits from a simple
/// shift register.
const MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

impl Rng {
    /// This function makes a new generator from a seed.
    ///
    /// A seed of 0 would trap the generator at state 0 forever, so this
    /// function swaps a seed of 0 for a fixed non-zero constant. Every
    /// other seed passes through with no change.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let state = if seed == 0 {
            ZERO_SEED_SUBSTITUTE
        } else {
            seed
        };
        Rng { state }
    }

    /// This function returns the next raw 64 bit output.
    ///
    /// The function runs one step of xorshift64* on the internal state,
    /// then multiplies the result by a fixed odd constant to spread the
    /// output bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(MULTIPLIER)
    }

    /// This function returns a float in the range 0.0 up to 1.0.
    ///
    /// The value never reaches 1.0. The function takes the top 53 bits
    /// of the raw output, because an `f64` mantissa holds only 53 bits.
    /// Extra low bits would not change the float value, so they would
    /// add no information and only cost time.
    pub fn next_f64(&mut self) -> f64 {
        let x = self.next_u64();
        (x >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// This function draws true with chance `p` and false with chance
    /// `1.0 - p`.
    ///
    /// The function always draws one `next_f64` value, even when `p` is
    /// at or below 0.0 or at or above 1.0. A fixed draw count on every
    /// call keeps the rest of the output stream the same, no matter
    /// what `p` is. This choice matters because a later change of `p`
    /// on one item must not shift the draws of every item after it.
    ///
    /// When `p <= 0.0` the function always returns false. When
    /// `p >= 1.0` the function always returns true.
    pub fn bernoulli(&mut self, p: f64) -> bool {
        let u = self.next_f64();
        if p <= 0.0 {
            false
        } else if p >= 1.0 {
            true
        } else {
            u < p
        }
    }

    /// This function draws a float in the range `low` up to `high`.
    ///
    /// The function does not check that `low` is at or below `high`.
    /// The caller must pass a valid range.
    pub fn uniform_range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next_f64()
    }

    /// This function draws an index in the range 0 up to `upper`.
    ///
    /// The function returns 0 when `upper` is 0, so the caller does not
    /// need to check for an empty range first.
    ///
    /// The function uses rejection sampling on `next_u64` output. A
    /// plain modulo would give a small bias toward low indexes when
    /// `upper` does not divide `u64::MAX + 1` evenly. Rejection
    /// sampling removes that bias at the cost of a rare extra draw.
    pub fn next_index(&mut self, upper: usize) -> usize {
        if upper == 0 {
            return 0;
        }
        let upper_u64 = upper as u64;
        // The largest multiple of `upper_u64` that fits in a u64. A
        // draw at or above this limit would give a biased remainder,
        // so the function throws that draw away and tries again.
        let limit = u64::MAX - (u64::MAX % upper_u64);
        loop {
            let x = self.next_u64();
            if x < limit {
                return (x % upper_u64) as usize;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This test locks in the first 5 outputs for seed 42. The values
    /// came from one run of `next_u64` on this exact algorithm. A
    /// change to the algorithm must not change these values by
    /// accident.
    #[test]
    fn known_answer_seed_42() {
        let mut rng = Rng::new(42);
        let got: Vec<u64> = (0..5).map(|_| rng.next_u64()).collect();
        assert_eq!(
            got,
            vec![
                6_255_019_084_209_693_600,
                14_430_073_426_741_505_498,
                14_575_455_857_230_217_846,
                17_414_512_882_241_728_735,
                14_100_574_548_354_140_678,
            ]
        );
    }

    #[test]
    fn next_f64_stays_in_range() {
        let mut rng = Rng::new(7);
        for _ in 0..100_000 {
            let v = rng.next_f64();
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn next_f64_mean_is_near_half() {
        let mut rng = Rng::new(1234);
        let n = 100_000;
        let sum: f64 = (0..n).map(|_| rng.next_f64()).sum();
        let mean = sum / f64::from(n);
        assert!((mean - 0.5).abs() < 0.01, "mean was {mean}");
    }

    #[test]
    fn zero_seed_does_not_stick() {
        let mut rng = Rng::new(0);
        let a = rng.next_u64();
        let b = rng.next_u64();
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn bernoulli_edge_cases_consume_one_draw() {
        let mut rng_a = Rng::new(99);
        let mut rng_b = Rng::new(99);
        // p <= 0.0 always gives false, but still draws one value, so
        // the two generators stay in step with each other.
        assert!(!rng_a.bernoulli(0.0));
        rng_b.next_f64();
        assert_eq!(rng_a.next_u64(), rng_b.next_u64());
    }

    #[test]
    fn bernoulli_p_one_is_always_true() {
        let mut rng = Rng::new(55);
        for _ in 0..1_000 {
            assert!(rng.bernoulli(1.0));
        }
    }

    #[test]
    fn bernoulli_p_zero_is_always_false() {
        let mut rng = Rng::new(56);
        for _ in 0..1_000 {
            assert!(!rng.bernoulli(0.0));
        }
    }

    #[test]
    fn next_index_covers_every_bucket_without_obvious_bias() {
        let mut rng = Rng::new(2024);
        let upper = 7;
        let mut counts = vec![0u32; upper];
        let draws = 100_000;
        for _ in 0..draws {
            let idx = rng.next_index(upper);
            assert!(idx < upper);
            counts[idx] += 1;
        }
        let expected = draws as f64 / upper as f64;
        for (bucket, &count) in counts.iter().enumerate() {
            let diff = (f64::from(count) - expected).abs() / expected;
            assert!(
                diff < 0.05,
                "bucket {bucket} was skewed: count {count}, expected {expected}"
            );
        }
    }

    #[test]
    fn next_index_zero_upper_returns_zero() {
        let mut rng = Rng::new(3);
        assert_eq!(rng.next_index(0), 0);
    }
}
