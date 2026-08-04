//! This module makes the ground truth data set for the simulator.
//!
//! The functions here draw a label and a signal value for each item.
//! The draws use the crate's own PRNG, so the same seed always gives
//! the same data set.

use crate::rng::Rng;
use crate::types::{Dataset, DatasetShape, Item};

/// This function makes a new data set.
///
/// The function makes `n` items with the given `shape` and `seed`. The
/// same `shape`, `n`, and `seed` always give the same data set, on
/// every machine.
///
/// Each item gets a label from a Bernoulli draw at the base rate of the
/// shape. Each item also gets a signal value:
///
/// - `Balanced` and `Skewed` draw the signal from a uniform range of
///   0.0 up to 1.0.
/// - `HardTail` makes 20 percent of items almost impossible. Those
///   items draw a signal from 0.0 up to 0.05. The rest draw a signal
///   from 0.0 up to 1.0, the same as `Balanced`.
///
/// A high signal value means an easy item. See `Item::signal` for this
/// rule.
///
/// The `Dataset` this function returns carries its own `seed`. Do not
/// reuse that value as the response seed for
/// `crate::archetype::responses_for`. See the trap note on that
/// function for the reason: the two seeds must differ, or the miner's
/// draws line up with the item's own draws and bias the result.
#[must_use]
pub fn generate(shape: DatasetShape, n: usize, seed: u64) -> Dataset {
    let mut rng = Rng::new(seed);
    let base_rate = shape.base_rate();

    let mut items = Vec::with_capacity(n);
    for index in 0..n {
        let label = u8::from(rng.bernoulli(base_rate));
        let signal = match shape {
            DatasetShape::Balanced | DatasetShape::Skewed => rng.uniform_range(0.0, 1.0),
            DatasetShape::HardTail => {
                let u = rng.next_f64();
                if u < 0.20 {
                    rng.uniform_range(0.0, 0.05)
                } else {
                    rng.uniform_range(0.0, 1.0)
                }
            }
        };
        items.push(Item {
            index,
            label,
            signal,
        });
    }

    let majority_label = compute_majority_label(&items);
    let hard_signal_threshold = compute_hard_signal_threshold(&items);
    let realised_base_rate = compute_realised_base_rate(&items);

    Dataset {
        shape,
        seed,
        items,
        majority_label,
        realised_base_rate,
        hard_signal_threshold,
    }
}

/// This function finds the label that occurs most often in `items`.
///
/// A tie gives label 1. An empty slice also gives label 1, because the
/// function starts the count of label 1 at a value that wins a 0-0 tie.
fn compute_majority_label(items: &[Item]) -> u8 {
    let mut count_zero: usize = 0;
    let mut count_one: usize = 0;
    for item in items {
        if item.label == 1 {
            count_one += 1;
        } else {
            count_zero += 1;
        }
    }
    if count_one >= count_zero {
        1
    } else {
        0
    }
}

/// This function counts the measured fraction of items with label 1.
///
/// The function returns 0.0 for an empty slice. There is no fraction of
/// no items. The `bayes_calibrated_good` archetype reads this value as
/// its prior. It stands for the class balance that a real miner reads
/// from the history of an intent, not the nominal target base rate of
/// the shape.
fn compute_realised_base_rate(items: &[Item]) -> f64 {
    if items.is_empty() {
        return 0.0;
    }
    let ones = items.iter().filter(|item| item.label == 1).count();
    ones as f64 / items.len() as f64
}

/// This function finds the 30th percentile of the signal values.
///
/// The function copies every signal value into a new vector, sorts the
/// vector with a stable sort and a total order, then reads the value at
/// index `n * 30 / 100`. The function returns 0.0 for an empty slice,
/// because there is no percentile of no values.
///
/// The function uses `f64::total_cmp` instead of `partial_cmp`. A
/// signal value is never NaN, but `total_cmp` gives a full order with
/// no risk of a panic, so the function uses it on principle.
fn compute_hard_signal_threshold(items: &[Item]) -> f64 {
    if items.is_empty() {
        return 0.0;
    }
    let mut signals: Vec<f64> = items.iter().map(|item| item.signal).collect();
    signals.sort_by(f64::total_cmp);
    let idx = signals.len() * 30 / 100;
    let idx = idx.min(signals.len() - 1);
    signals[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_base_rate_is_near_half() {
        let dataset = generate(DatasetShape::Balanced, 10_000, 1);
        let ones = dataset.items.iter().filter(|item| item.label == 1).count();
        let rate = ones as f64 / 10_000.0;
        assert!((rate - 0.50).abs() < 0.02, "rate was {rate}");
    }

    #[test]
    fn skewed_base_rate_is_near_ninety_percent() {
        let dataset = generate(DatasetShape::Skewed, 10_000, 2);
        let ones = dataset.items.iter().filter(|item| item.label == 1).count();
        let rate = ones as f64 / 10_000.0;
        assert!((rate - 0.90).abs() < 0.02, "rate was {rate}");
    }

    #[test]
    fn hard_tail_has_near_twenty_percent_almost_impossible_items() {
        // The generator picks the near-zero branch for 20 percent of
        // items. But the other branch draws from 0.0 up to 1.0, so it
        // also lands under 0.05 about 5 percent of the time. The real
        // share of items with a signal under 0.05 is near
        // 0.20 + 0.80 * 0.05 = 0.24, not 0.20. This test checks the
        // real mixed rate, not the rate of the near-zero branch alone.
        let dataset = generate(DatasetShape::HardTail, 10_000, 3);
        let hard = dataset
            .items
            .iter()
            .filter(|item| item.signal < 0.05)
            .count();
        let rate = hard as f64 / 10_000.0;
        let expected = 0.20 + 0.80 * 0.05;
        assert!(
            (rate - expected).abs() < 0.02,
            "rate was {rate}, expected near {expected}"
        );
    }

    #[test]
    fn majority_label_matches_a_hand_built_case() {
        let items = vec![
            Item {
                index: 0,
                label: 1,
                signal: 0.5,
            },
            Item {
                index: 1,
                label: 1,
                signal: 0.5,
            },
            Item {
                index: 2,
                label: 0,
                signal: 0.5,
            },
        ];
        assert_eq!(compute_majority_label(&items), 1);

        let tied = vec![
            Item {
                index: 0,
                label: 1,
                signal: 0.5,
            },
            Item {
                index: 1,
                label: 0,
                signal: 0.5,
            },
        ];
        assert_eq!(compute_majority_label(&tied), 1, "a tie must give label 1");
    }

    #[test]
    fn same_seed_gives_identical_dataset() {
        let a = generate(DatasetShape::Balanced, 500, 42);
        let b = generate(DatasetShape::Balanced, 500, 42);
        assert_eq!(a.majority_label, b.majority_label);
        assert_eq!(a.hard_signal_threshold, b.hard_signal_threshold);
        assert_eq!(a.realised_base_rate, b.realised_base_rate);
        for (x, y) in a.items.iter().zip(b.items.iter()) {
            assert_eq!(x.label, y.label);
            assert_eq!(x.signal, y.signal);
        }
    }

    #[test]
    fn different_seed_gives_different_dataset() {
        let a = generate(DatasetShape::Balanced, 500, 42);
        let b = generate(DatasetShape::Balanced, 500, 43);
        let same = a
            .items
            .iter()
            .zip(b.items.iter())
            .all(|(x, y)| x.signal == y.signal);
        assert!(!same, "two different seeds gave the same signals");
    }

    #[test]
    fn zero_items_does_not_panic() {
        let dataset = generate(DatasetShape::Balanced, 0, 1);
        assert!(dataset.items.is_empty());
        assert_eq!(dataset.hard_signal_threshold, 0.0);
        assert_eq!(dataset.majority_label, 1);
        assert_eq!(dataset.realised_base_rate, 0.0);
    }

    #[test]
    fn realised_base_rate_matches_the_target_within_sampling_error() {
        let dataset = generate(DatasetShape::Skewed, 20_000, 21);
        let target = DatasetShape::Skewed.base_rate();
        assert!(
            (dataset.realised_base_rate - target).abs() < 0.02,
            "realised base rate was {}, target was {target}",
            dataset.realised_base_rate
        );
    }

    #[test]
    fn realised_base_rate_is_the_exact_measured_fraction() {
        // 997 is prime, so 90 percent of 997 items is not a whole
        // number. The realised base rate can never equal the nominal
        // target 0.90 exactly for this item count. This makes the
        // "not the nominal rate" check below true every time, not just
        // by luck.
        let dataset = generate(DatasetShape::Skewed, 997, 22);
        let ones = dataset.items.iter().filter(|item| item.label == 1).count();
        let expected = ones as f64 / 997.0;
        assert_eq!(dataset.realised_base_rate, expected);
        assert_ne!(
            dataset.realised_base_rate,
            DatasetShape::Skewed.base_rate(),
            "realised base rate must be the measured fraction, not the nominal shape base rate"
        );
    }
}
