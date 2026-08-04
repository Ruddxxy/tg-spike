//! This module builds and prints the leaderboard.
//!
//! A leaderboard ranks the miners by mean score. Rank 1 is the best
//! miner, because a HIGH score is good. Read the note on score
//! direction in `types.rs`.
//!
//! This module also builds `Standings`, which is the Layer 2 view of a
//! set of miner results. `Standings` applies one of the two aggregation
//! models: `ScoreAndKeep` keeps every miner ranked, and `Eject` removes
//! a miner at its first failure. Read the note on the two layers in
//! `types.rs`.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::types::{
    AggregationModel, Archetype, EjectedRow, LeaderboardRow, MinerResult, Standings,
};

/// This function finds the mean of a list of values.
///
/// The function sorts a copy of `values` first, with `f64::total_cmp`.
/// A stable sort with an explicit total order removes the effect of
/// input order on the sum: two callers that pass the same values in a
/// different order get the exact same mean. The function then adds the
/// sorted values with Kahan compensated summation, which cuts the
/// rounding error of a plain running sum. This crate never calls
/// `partial_cmp().unwrap()`; `f64::total_cmp` gives a total order even
/// when a value is `NaN`, though a `NaN` score should never reach this
/// function.
///
/// The function returns 0.0 for an empty list.
#[must_use]
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(f64::total_cmp);

    let mut sum = 0.0;
    let mut compensation = 0.0;
    for &value in &sorted {
        let adjusted = value - compensation;
        let new_sum = sum + adjusted;
        compensation = (new_sum - sum) - adjusted;
        sum = new_sum;
    }
    sum / sorted.len() as f64
}

/// This function finds the median of a list of values.
///
/// The function sorts a copy of `values` with `f64::total_cmp`. For an
/// odd count it returns the middle value. For an even count it returns
/// the mean of the two middle values.
///
/// The function returns 0.0 for an empty list.
#[must_use]
pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// This function builds a leaderboard from a list of miner results.
///
/// The function ranks the miners in DESCENDING order of mean score,
/// because a HIGH score is good. Rank 1 is the best miner.
///
/// The function breaks a tie with a fixed order: first by `mean_score`
/// with `f64::total_cmp`, then by `median_score` with `f64::total_cmp`,
/// then by `archetype`, ascending. The archetype tiebreak is arbitrary,
/// but it is fixed and does not depend on input order, so a tie never
/// falls on luck.
///
/// This function never ejects a miner. Every entry in `results` gets a
/// row and a rank. Call [`build_standings`] with `AggregationModel::Eject`
/// to apply the whitepaper ejection rule.
#[must_use]
pub fn build(results: &[MinerResult]) -> Vec<LeaderboardRow> {
    let mut rows: Vec<LeaderboardRow> = results
        .iter()
        .map(|result| LeaderboardRow {
            rank: 0,
            archetype: result.archetype,
            mean_score: mean(&result.scores),
            median_score: median(&result.scores),
            n_malformed: result.n_malformed,
            n_abstained: result.n_abstained,
        })
        .collect();

    rows.sort_by(|a, b| {
        b.mean_score
            .total_cmp(&a.mean_score)
            .then_with(|| b.median_score.total_cmp(&a.median_score))
            .then_with(|| a.archetype.cmp(&b.archetype))
    });

    for (i, row) in rows.iter_mut().enumerate() {
        row.rank = i + 1;
    }
    rows
}

/// This function builds the `Standings` of a list of miner results under
/// one aggregation model.
///
/// - `AggregationModel::ScoreAndKeep` ranks every miner with [`build`].
///   `ejected` is always empty for this model. This model is NOT the
///   protocol rule. The simulator keeps it only for comparison.
/// - `AggregationModel::Eject` removes every miner with a
///   `first_failure` from the ranking pool, and ranks the rest with
///   [`build`]. Read the whitepaper v1.0, section 5.1. An ejected miner
///   gets no rank at all. It does NOT get a score of 0.0 and last
///   place; that is a different rule, and this crate does not apply it.
///
/// The `ejected` list is sorted by archetype, so its order does not
/// depend on the order of `results`.
#[must_use]
pub fn build_standings(results: &[MinerResult], model: AggregationModel) -> Standings {
    match model {
        AggregationModel::ScoreAndKeep => Standings {
            model,
            ranked: build(results),
            ejected: Vec::new(),
        },
        AggregationModel::Eject => {
            let kept: Vec<MinerResult> = results
                .iter()
                .filter(|result| result.first_failure.is_none())
                .cloned()
                .collect();

            let mut ejected: Vec<EjectedRow> = results
                .iter()
                .filter_map(|result| {
                    result.first_failure.map(|failure| EjectedRow {
                        archetype: result.archetype,
                        first_failure_index: failure.index,
                        reason: failure.reason,
                    })
                })
                .collect();
            ejected.sort_by(|a, b| a.archetype.cmp(&b.archetype));

            Standings {
                model,
                ranked: build(&kept),
                ejected,
            }
        }
    }
}

/// This function makes a fixed width text table of a leaderboard.
///
/// The table has these columns: rank, archetype, mean score, median
/// score, count of malformed responses, count of abstained responses.
/// Every float shows 6 digits after the decimal point. A HIGH mean
/// score is good.
#[must_use]
pub fn render(rows: &[LeaderboardRow], title: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{title}");
    let _ = writeln!(
        out,
        "{:<4} {:<20} {:>12} {:>12} {:>12} {:>12}",
        "rank", "archetype", "mean_score", "median", "n_malformed", "n_abstained"
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{:<4} {:<20} {:>12.6} {:>12.6} {:>12} {:>12}",
            row.rank,
            row.archetype.name(),
            row.mean_score,
            row.median_score,
            row.n_malformed,
            row.n_abstained
        );
    }
    out
}

/// This function makes a text table of one `Standings` value.
///
/// The table shows the ranked miners first, with the same columns as
/// [`render`]. When the model ejected at least one miner, the table
/// also shows an `EJECTED` section, with the archetype, the reason, and
/// the item index of the first failure.
#[must_use]
pub fn render_standings(standings: &Standings, title: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{title} [model: {}]", standings.model.name());
    let _ = write!(out, "{}", render(&standings.ranked, "ranked"));
    let _ = write!(
        out,
        "{}",
        render_ejected(standings, "ejected from the pool for the epoch")
    );
    out
}

/// This function makes a text table of the ejected miners of one
/// `Standings` value.
///
/// The table shows the archetype, the reason, and the item index of the
/// first failure. When no miner was ejected, the function prints one
/// line that says so, instead of an empty table.
#[must_use]
pub fn render_ejected(standings: &Standings, title: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{title}");
    if standings.ejected.is_empty() {
        let _ = writeln!(out, "no miner was ejected.");
        return out;
    }
    let _ = writeln!(
        out,
        "{:<22} {:<20} {:>19}",
        "archetype", "reason", "first_failure_index"
    );
    for row in &standings.ejected {
        let _ = writeln!(
            out,
            "{:<22} {:<20} {:>19}",
            row.archetype.name(),
            row.reason.name(),
            row.first_failure_index
        );
    }
    out
}

/// This function makes a side by side text table of the `ScoreAndKeep`
/// standings and the `Eject` standings of the same miner results.
///
/// The table has one row per archetype that stays ranked under
/// `score_and_keep`. Each row shows the rank and mean score under
/// `score_and_keep`, next to the rank and mean score under `eject`. A
/// row for an archetype that `eject` removed shows `EJECTED` in place
/// of a rank and a mean score.
#[must_use]
pub fn render_standings_side_by_side(
    score_and_keep: &Standings,
    eject: &Standings,
    title: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{title}");
    let _ = writeln!(
        out,
        "{:<22} {:>10} {:>12} {:>10} {:>12}  status",
        "archetype", "sak_rank", "sak_mean", "ej_rank", "ej_mean"
    );
    for row in &score_and_keep.ranked {
        match eject.ranked.iter().find(|r| r.archetype == row.archetype) {
            Some(ej_row) => {
                let _ = writeln!(
                    out,
                    "{:<22} {:>10} {:>12.6} {:>10} {:>12.6}  ranked in both models",
                    row.archetype.name(),
                    row.rank,
                    row.mean_score,
                    ej_row.rank,
                    ej_row.mean_score
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "{:<22} {:>10} {:>12.6} {:>10} {:>12}  EJECTED under the eject model",
                    row.archetype.name(),
                    row.rank,
                    row.mean_score,
                    "-",
                    "-"
                );
            }
        }
    }
    out
}

/// This function compares the order of the non-ejected miners between
/// the `ScoreAndKeep` standings and the `Eject` standings.
///
/// The function removes every archetype that `eject` ejected from the
/// `score_and_keep` rank order, then compares the remaining sequence to
/// the `eject` rank order. This tells a reader whether the aggregation
/// layer changes the RELATIVE ORDER of the miners that stay in the
/// pool, or whether it only removes rows and shifts rank numbers.
///
/// The function prints a table of the rank number of each non-ejected
/// archetype under each model, so a reader can see the size of the
/// shift even when the order itself does not change.
#[must_use]
pub fn compare_orderings(score_and_keep: &Standings, eject: &Standings) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "ORDER COMPARISON: non-ejected miners, score_and_keep vs eject"
    );

    let ejected_archetypes: BTreeSet<Archetype> =
        eject.ejected.iter().map(|row| row.archetype).collect();

    let kept_order: Vec<Archetype> = score_and_keep
        .ranked
        .iter()
        .filter(|row| !ejected_archetypes.contains(&row.archetype))
        .map(|row| row.archetype)
        .collect();
    let eject_order: Vec<Archetype> = eject.ranked.iter().map(|row| row.archetype).collect();

    if kept_order == eject_order {
        let _ = writeln!(
            out,
            "the sequence order of the non-ejected miners is the SAME in both models."
        );
        let _ = writeln!(
            out,
            "ejection removes rows from the list. it does not move any other row."
        );
    } else {
        let _ = writeln!(
            out,
            "the sequence order of the non-ejected miners DIFFERS between the two models."
        );
    }

    let _ = writeln!(
        out,
        "{:<22} {:>19} {:>10}",
        "archetype", "rank_score_and_keep", "rank_eject"
    );
    for archetype in &eject_order {
        let sak_rank = rank_of_archetype(&score_and_keep.ranked, *archetype);
        let ej_rank = rank_of_archetype(&eject.ranked, *archetype);
        let _ = writeln!(
            out,
            "{:<22} {:>19} {:>10}",
            archetype.name(),
            sak_rank,
            ej_rank
        );
    }
    out
}

/// This function finds the rank of one archetype in a leaderboard.
///
/// The function returns `None` when the archetype is not in `rows`.
fn rank_of(rows: &[LeaderboardRow], archetype: Archetype) -> Option<usize> {
    rows.iter()
        .find(|row| row.archetype == archetype)
        .map(|row| row.rank)
}

/// This function makes a side by side text table of a Brier leaderboard
/// and a log loss leaderboard.
///
/// The table has one row per archetype found in `brier`. Each row shows
/// the Brier rank and score next to the log loss rank and score for the
/// same archetype. When the two ranks differ, the function marks the
/// row with a clear tag, so a reader can see where the two metrics do
/// not agree.
///
/// An archetype that is in `brier` but not in `log_loss` gets a marked
/// row that says the archetype is missing, instead of a panic.
#[must_use]
pub fn render_side_by_side(
    brier: &[LeaderboardRow],
    log_loss: &[LeaderboardRow],
    title: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{title}");
    let _ = writeln!(
        out,
        "{:<20} {:>10} {:>12} {:>10} {:>12}  note",
        "archetype", "brier_rank", "brier_mean", "ll_rank", "ll_mean"
    );
    for brier_row in brier {
        let archetype = brier_row.archetype;
        match log_loss.iter().find(|row| row.archetype == archetype) {
            Some(ll_row) => {
                let note = if brier_row.rank != ll_row.rank {
                    format!(
                        "<-- RANK DIFFERS (brier #{}, logloss #{})",
                        brier_row.rank, ll_row.rank
                    )
                } else {
                    String::new()
                };
                let _ = writeln!(
                    out,
                    "{:<20} {:>10} {:>12.6} {:>10} {:>12.6}  {}",
                    archetype.name(),
                    brier_row.rank,
                    brier_row.mean_score,
                    ll_row.rank,
                    ll_row.mean_score,
                    note
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "{:<20} {:>10} {:>12.6} {:>10} {:>12}  <-- MISSING FROM LOG LOSS TABLE",
                    archetype.name(),
                    brier_row.rank,
                    brier_row.mean_score,
                    "-",
                    "-"
                );
            }
        }
    }
    out
}

/// This function gives the rank of an archetype, or 0 if it is absent.
///
/// This helper exists for callers outside this module that need a
/// quick rank lookup, for example the summary code in `main.rs`.
#[must_use]
pub fn rank_of_archetype(rows: &[LeaderboardRow], archetype: Archetype) -> usize {
    rank_of(rows, archetype).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EjectionReason, Failure, MinerResult};

    fn result(archetype: Archetype, scores: Vec<f64>) -> MinerResult {
        MinerResult {
            archetype,
            scores,
            n_malformed: 0,
            n_abstained: 0,
            first_failure: None,
        }
    }

    fn failing_result(archetype: Archetype, scores: Vec<f64>, failure: Failure) -> MinerResult {
        MinerResult {
            archetype,
            scores,
            n_malformed: 0,
            n_abstained: 0,
            first_failure: Some(failure),
        }
    }

    #[test]
    fn mean_of_simple_values() {
        let got = mean(&[1.0, 2.0, 3.0, 4.0]);
        assert!((got - 2.5).abs() < 1e-12);
    }

    #[test]
    fn mean_of_empty_is_zero() {
        assert_eq!(mean(&[]), 0.0);
    }

    #[test]
    fn mean_does_not_depend_on_input_order() {
        let a = mean(&[0.1, 0.2, 0.3, 0.4, 0.5]);
        let b = mean(&[0.5, 0.1, 0.4, 0.2, 0.3]);
        assert_eq!(a, b);
    }

    #[test]
    fn median_of_odd_count() {
        let got = median(&[3.0, 1.0, 2.0]);
        assert!((got - 2.0).abs() < 1e-12);
    }

    #[test]
    fn median_of_even_count() {
        let got = median(&[1.0, 2.0, 3.0, 4.0]);
        assert!((got - 2.5).abs() < 1e-12);
    }

    #[test]
    fn median_of_empty_is_zero() {
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn build_ranks_higher_mean_first() {
        let results = vec![
            result(Archetype::Random, vec![0.5, 0.5]),
            result(Archetype::Oracle, vec![0.99, 0.99]),
            result(Archetype::Contrarian, vec![0.1, 0.1]),
        ];
        let rows = build(&results);
        assert_eq!(rows[0].archetype, Archetype::Oracle);
        assert_eq!(rows[0].rank, 1);
        assert_eq!(rows[1].archetype, Archetype::Random);
        assert_eq!(rows[1].rank, 2);
        assert_eq!(rows[2].archetype, Archetype::Contrarian);
        assert_eq!(rows[2].rank, 3);
    }

    #[test]
    fn build_breaks_ties_deterministically_by_archetype_order() {
        // Oracle and NoisyGood tie on mean and median. Oracle has a
        // smaller enum discriminant order (it comes first in `ALL`),
        // so `Archetype::cmp` must place it first.
        let results = vec![
            result(Archetype::NoisyGood, vec![0.8, 0.8]),
            result(Archetype::Oracle, vec![0.8, 0.8]),
        ];
        let rows_a = build(&results);
        let mut reversed = results.clone();
        reversed.reverse();
        let rows_b = build(&reversed);

        assert_eq!(rows_a[0].archetype, Archetype::Oracle);
        assert_eq!(rows_b[0].archetype, Archetype::Oracle);
        assert_eq!(rows_a[0].rank, rows_b[0].rank);
    }

    #[test]
    fn build_standings_score_and_keep_never_ejects() {
        let results = vec![
            failing_result(
                Archetype::Abstainer,
                vec![0.0, 0.9],
                Failure {
                    index: 0,
                    reason: EjectionReason::NoResponse,
                },
            ),
            result(Archetype::Oracle, vec![0.99, 0.99]),
        ];
        let standings = build_standings(&results, AggregationModel::ScoreAndKeep);
        assert!(standings.ejected.is_empty());
        assert_eq!(standings.ranked.len(), 2);
    }

    #[test]
    fn build_standings_eject_removes_the_first_failure_miner() {
        let results = vec![
            failing_result(
                Archetype::Abstainer,
                vec![0.0, 0.9],
                Failure {
                    index: 0,
                    reason: EjectionReason::NoResponse,
                },
            ),
            result(Archetype::Oracle, vec![0.99, 0.99]),
        ];
        let standings = build_standings(&results, AggregationModel::Eject);
        assert_eq!(standings.ejected.len(), 1);
        assert_eq!(standings.ejected[0].archetype, Archetype::Abstainer);
        assert_eq!(standings.ejected[0].first_failure_index, 0);
        assert_eq!(standings.ejected[0].reason, EjectionReason::NoResponse);
        assert_eq!(standings.ranked.len(), 1);
        assert_eq!(standings.ranked[0].archetype, Archetype::Oracle);
        assert_eq!(standings.ranked[0].rank, 1);
    }

    #[test]
    fn compare_orderings_reports_the_same_sequence_when_ejection_only_removes_rows() {
        let results = vec![
            result(Archetype::Oracle, vec![0.99, 0.99]),
            failing_result(
                Archetype::Abstainer,
                vec![0.0, 0.9],
                Failure {
                    index: 0,
                    reason: EjectionReason::NoResponse,
                },
            ),
            result(Archetype::Random, vec![0.5, 0.5]),
        ];
        let sak = build_standings(&results, AggregationModel::ScoreAndKeep);
        let eject = build_standings(&results, AggregationModel::Eject);
        let text = compare_orderings(&sak, &eject);
        assert!(text.contains("the SAME in both models"));
    }

    #[test]
    fn render_side_by_side_marks_rank_disagreement() {
        let brier = vec![
            LeaderboardRow {
                rank: 1,
                archetype: Archetype::Oracle,
                mean_score: 0.99,
                median_score: 0.99,
                n_malformed: 0,
                n_abstained: 0,
            },
            LeaderboardRow {
                rank: 2,
                archetype: Archetype::NoisyGood,
                mean_score: 0.9,
                median_score: 0.9,
                n_malformed: 0,
                n_abstained: 0,
            },
        ];
        let log_loss = vec![
            LeaderboardRow {
                rank: 2,
                archetype: Archetype::Oracle,
                mean_score: 0.98,
                median_score: 0.98,
                n_malformed: 0,
                n_abstained: 0,
            },
            LeaderboardRow {
                rank: 1,
                archetype: Archetype::NoisyGood,
                mean_score: 0.99,
                median_score: 0.99,
                n_malformed: 0,
                n_abstained: 0,
            },
        ];
        let text = render_side_by_side(&brier, &log_loss, "test");
        assert!(text.contains("RANK DIFFERS"));
    }
}
