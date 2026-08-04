//! This module checks the seven order invariants of the leaderboard.
//!
//! Each invariant states a known fact about the true quality of two
//! archetypes. A correct scoring rule must keep every invariant. A
//! failed invariant is a real finding about the scoring rule, not a bug
//! in this crate to hide.
//!
//! This module checks two different rule sets:
//!
//! - [`check_all`] checks a plain leaderboard, built under the
//!   `ScoreAndKeep` aggregation model. Every invariant here is a claim
//!   about RANK NUMBERS.
//! - [`check_all_eject`] checks a `Standings` value, built under the
//!   `Eject` aggregation model. Invariants 1 to 5 are still claims about
//!   rank numbers, over the miners that stay in the pool. Invariants 6
//!   and 7 are NOT claims about rank numbers any more. They are claims
//!   about the AGGREGATION layer: that the protocol removed a failing
//!   miner from the pool, at its first failure. Read the note on the
//!   two layers in `types.rs`.

use std::fmt::Write as _;

use crate::types::{Archetype, EjectionReason, LeaderboardRow, Standings, VerdictLine};

/// This function finds the rank of an archetype in a leaderboard.
///
/// The function returns an error text instead of a rank when the
/// archetype is not in `rows`. The caller uses the error text as the
/// detail of a failed invariant.
fn find_rank(rows: &[LeaderboardRow], archetype: Archetype) -> Result<usize, String> {
    rows.iter()
        .find(|row| row.archetype == archetype)
        .map(|row| row.rank)
        .ok_or_else(|| {
            format!(
                "archetype {} is missing from the leaderboard",
                archetype.name()
            )
        })
}

/// This function finds the mean score of an archetype in a leaderboard.
fn find_mean(rows: &[LeaderboardRow], archetype: Archetype) -> Option<f64> {
    rows.iter()
        .find(|row| row.archetype == archetype)
        .map(|row| row.mean_score)
}

/// This function checks that `above` has a smaller rank number than
/// `below`. A smaller rank number is a better rank, because a HIGH
/// score is good, and rank 1 always goes to the highest mean score.
///
/// The function builds a detail string with the actual rank and mean
/// score of both archetypes, whether the check passes or fails.
fn check_above(
    rows: &[LeaderboardRow],
    number: usize,
    statement: &'static str,
    above: Archetype,
    below: Archetype,
    gaming_vector: bool,
) -> VerdictLine {
    let above_rank = find_rank(rows, above);
    let below_rank = find_rank(rows, below);

    match (above_rank, below_rank) {
        (Ok(above_rank), Ok(below_rank)) => {
            let above_mean = find_mean(rows, above).unwrap_or(f64::NAN);
            let below_mean = find_mean(rows, below).unwrap_or(f64::NAN);
            let passed = above_rank < below_rank;
            let detail = format!(
                "{} rank {} mean {:.6} vs {} rank {} mean {:.6}",
                above.name(),
                above_rank,
                above_mean,
                below.name(),
                below_rank,
                below_mean
            );
            VerdictLine {
                number,
                statement,
                passed,
                detail,
                gaming_vector,
            }
        }
        (Err(e), _) | (_, Err(e)) => VerdictLine {
            number,
            statement,
            passed: false,
            detail: e,
            gaming_vector,
        },
    }
}

/// This function checks that `at_or_above` does not have a smaller rank
/// number than `reference`. This form is for invariant 6 of `check_all`:
/// the abstainer must not out-rank `noisy_good`, but it may tie or fall
/// below it.
fn check_not_above(
    rows: &[LeaderboardRow],
    number: usize,
    statement: &'static str,
    at_or_above: Archetype,
    reference: Archetype,
    gaming_vector: bool,
) -> VerdictLine {
    let a_rank = find_rank(rows, at_or_above);
    let r_rank = find_rank(rows, reference);

    match (a_rank, r_rank) {
        (Ok(a_rank), Ok(r_rank)) => {
            let a_mean = find_mean(rows, at_or_above).unwrap_or(f64::NAN);
            let r_mean = find_mean(rows, reference).unwrap_or(f64::NAN);
            let passed = a_rank >= r_rank;
            let detail = format!(
                "{} rank {} mean {:.6} vs {} rank {} mean {:.6}",
                at_or_above.name(),
                a_rank,
                a_mean,
                reference.name(),
                r_rank,
                r_mean
            );
            VerdictLine {
                number,
                statement,
                passed,
                detail,
                gaming_vector,
            }
        }
        (Err(e), _) | (_, Err(e)) => VerdictLine {
            number,
            statement,
            passed: false,
            detail: e,
            gaming_vector,
        },
    }
}

/// This function checks that the protocol ejected `archetype`, and that
/// it ejected it for `expected_reason`.
///
/// The function fails the check when `archetype` still has a rank
/// (the protocol did not eject it), when `archetype` is missing from
/// both lists, or when the recorded reason does not match
/// `expected_reason`. The function does not check that the recorded
/// `first_failure_index` really is the FIRST failing item. That check
/// needs the raw response list, which `Standings` does not carry. Read
/// the integration test `tests/invariants.rs` for that check.
fn check_ejected(
    standings: &Standings,
    number: usize,
    statement: &'static str,
    archetype: Archetype,
    expected_reason: EjectionReason,
    gaming_vector: bool,
) -> VerdictLine {
    let still_ranked = standings
        .ranked
        .iter()
        .any(|row| row.archetype == archetype);
    let ejected_row = standings
        .ejected
        .iter()
        .find(|row| row.archetype == archetype);

    match (still_ranked, ejected_row) {
        (false, Some(row)) => {
            let passed = row.reason == expected_reason;
            let detail = format!(
                "{} ejected at item index {}, reason {} (expected reason {})",
                archetype.name(),
                row.first_failure_index,
                row.reason.name(),
                expected_reason.name()
            );
            VerdictLine {
                number,
                statement,
                passed,
                detail,
                gaming_vector,
            }
        }
        (true, _) => VerdictLine {
            number,
            statement,
            passed: false,
            detail: format!(
                "{} still has a rank. the eject model did not remove it from the pool.",
                archetype.name()
            ),
            gaming_vector,
        },
        (false, None) => VerdictLine {
            number,
            statement,
            passed: false,
            detail: format!(
                "{} is missing from both the ranked list and the ejected list.",
                archetype.name()
            ),
            gaming_vector,
        },
    }
}

/// This function runs invariants 1 to 5 on one leaderboard.
///
/// These five checks are the same under both aggregation models: a
/// smaller rank number is a better rank, because a HIGH score is good.
/// [`check_all`] and [`check_all_eject`] both start from this list, then
/// append their own versions of invariants 6 and 7.
fn rank_invariants(rows: &[LeaderboardRow]) -> Vec<VerdictLine> {
    let oracle_first = {
        let oracle_rank = find_rank(rows, Archetype::Oracle);
        match oracle_rank {
            Ok(1) => VerdictLine {
                number: 1,
                statement: "oracle ranks first",
                passed: true,
                detail: format!(
                    "{} rank 1 mean {:.6}",
                    Archetype::Oracle.name(),
                    find_mean(rows, Archetype::Oracle).unwrap_or(f64::NAN)
                ),
                gaming_vector: false,
            },
            Ok(rank) => VerdictLine {
                number: 1,
                statement: "oracle ranks first",
                passed: false,
                detail: format!(
                    "{} rank {} mean {:.6}, expected rank 1",
                    Archetype::Oracle.name(),
                    rank,
                    find_mean(rows, Archetype::Oracle).unwrap_or(f64::NAN)
                ),
                gaming_vector: false,
            },
            Err(e) => VerdictLine {
                number: 1,
                statement: "oracle ranks first",
                passed: false,
                detail: e,
                gaming_vector: false,
            },
        }
    };

    vec![
        oracle_first,
        check_above(
            rows,
            2,
            "noisy_good ranks above noisy_mediocre",
            Archetype::NoisyGood,
            Archetype::NoisyMediocre,
            false,
        ),
        check_above(
            rows,
            3,
            "noisy_good ranks above constant_majority",
            Archetype::NoisyGood,
            Archetype::ConstantMajority,
            true,
        ),
        check_above(
            rows,
            4,
            "noisy_good ranks above overconfident_good",
            Archetype::NoisyGood,
            Archetype::OverconfidentGood,
            true,
        ),
        check_above(
            rows,
            5,
            "contrarian ranks below random",
            Archetype::Random,
            Archetype::Contrarian,
            false,
        ),
    ]
}

/// This function runs the seven order invariant checks on one
/// leaderboard, built under the `ScoreAndKeep` aggregation model.
///
/// The checks run in this fixed order:
///
/// 1. oracle ranks first
/// 2. `noisy_good` ranks above `noisy_mediocre`
/// 3. `noisy_good` ranks above `constant_majority` (gaming vector)
/// 4. `noisy_good` ranks above `overconfident_good` (gaming vector)
/// 5. contrarian ranks below random
/// 6. abstainer does not rank above `noisy_good` (gaming vector)
/// 7. malformer ranks below `noisy_good`
///
/// "Ranks above" means a smaller rank number, so a better mean score.
/// Every line carries the actual rank and mean score of both
/// archetypes, whether the check passes or fails. An archetype missing
/// from `rows` fails its line with a clear reason instead of a panic.
///
/// `ScoreAndKeep` is NOT the protocol rule. It keeps a failing miner
/// ranked, instead of removing it from the pool. Call [`check_all_eject`]
/// to check the standings under the real protocol rule.
#[must_use]
pub fn check_all(rows: &[LeaderboardRow]) -> Vec<VerdictLine> {
    let mut lines = rank_invariants(rows);
    lines.push(check_not_above(
        rows,
        6,
        "abstainer does not rank above noisy_good",
        Archetype::Abstainer,
        Archetype::NoisyGood,
        true,
    ));
    lines.push(check_above(
        rows,
        7,
        "malformer ranks below noisy_good",
        Archetype::NoisyGood,
        Archetype::Malformer,
        false,
    ));
    lines
}

/// This function runs the seven order invariant checks on one
/// `Standings` value, built under the `Eject` aggregation model.
///
/// Invariants 1 to 5 are the same checks as [`check_all`], run over the
/// miners that `standings.ranked` still holds. Invariants 6 and 7 are
/// restated for this model, because a failing miner does not keep a
/// rank here:
///
/// 6. abstainer is ejected, and the ejection happens at the first
///    abstention (gaming vector)
/// 7. malformer is ejected, and the ejection happens at the first
///    malformed response
///
/// Invariants 6 and 7 test the AGGREGATION layer of this crate, not the
/// scoring rule of the WASM module. A pass here says the protocol
/// removed the miner from the routing pool. It does not say anything
/// about the size of any score.
#[must_use]
pub fn check_all_eject(standings: &Standings) -> Vec<VerdictLine> {
    let mut lines = rank_invariants(&standings.ranked);
    lines.push(check_ejected(
        standings,
        6,
        "abstainer is ejected, at its first abstention",
        Archetype::Abstainer,
        EjectionReason::NoResponse,
        true,
    ));
    lines.push(check_ejected(
        standings,
        7,
        "malformer is ejected, at its first malformed response",
        Archetype::Malformer,
        EjectionReason::MalformedResponse,
        false,
    ));
    lines
}

/// This function makes a text report of a list of verdict lines.
///
/// Each line shows PASS or FAIL, the invariant number, and the
/// statement. A gaming vector line gets a `[GAMING VECTOR]` tag, so it
/// stands out even when it passes. The detail numbers follow on the
/// same line.
#[must_use]
pub fn render(lines: &[VerdictLine], title: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{title}");
    for line in lines {
        let status = if line.passed { "PASS" } else { "FAIL" };
        let tag = if line.gaming_vector {
            " [GAMING VECTOR]"
        } else {
            ""
        };
        let _ = writeln!(out, "{} {}. {}{}", status, line.number, line.statement, tag);
        let _ = writeln!(out, "     {}", line.detail);
    }
    out
}

/// This function tells if every verdict line passed.
#[must_use]
pub fn all_passed(lines: &[VerdictLine]) -> bool {
    lines.iter().all(|line| line.passed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AggregationModel, EjectedRow};

    fn row(archetype: Archetype, rank: usize, mean_score: f64) -> LeaderboardRow {
        LeaderboardRow {
            rank,
            archetype,
            mean_score,
            median_score: mean_score,
            n_malformed: 0,
            n_abstained: 0,
        }
    }

    /// This is a leaderboard where every invariant should pass. A HIGH
    /// mean score is good, so the best miner (oracle) has the highest
    /// mean score and rank 1.
    fn healthy_rows() -> Vec<LeaderboardRow> {
        vec![
            row(Archetype::Oracle, 1, 0.99),
            row(Archetype::NoisyGood, 2, 0.90),
            row(Archetype::UnderconfidentGood, 3, 0.88),
            row(Archetype::NoisyMediocre, 4, 0.80),
            row(Archetype::OverconfidentGood, 5, 0.75),
            row(Archetype::Abstainer, 6, 0.72),
            row(Archetype::Malformer, 7, 0.70),
            row(Archetype::ConstantMajority, 8, 0.50),
            row(Archetype::Random, 9, 0.45),
            row(Archetype::Contrarian, 10, 0.10),
        ]
    }

    #[test]
    fn healthy_leaderboard_passes_every_invariant() {
        let lines = check_all(&healthy_rows());
        assert!(all_passed(&lines));
        assert_eq!(lines.len(), 7);
    }

    #[test]
    fn oracle_not_first_fails_invariant_1() {
        let mut rows = healthy_rows();
        rows[0].rank = 2;
        rows[1].rank = 1;
        let lines = check_all(&rows);
        assert!(!lines[0].passed);
    }

    #[test]
    fn noisy_good_below_constant_majority_fails_invariant_3() {
        let mut rows = healthy_rows();
        // Swap ranks so constant_majority beats noisy_good.
        let ng = rows
            .iter()
            .position(|r| r.archetype == Archetype::NoisyGood)
            .expect("noisy_good present");
        let cm = rows
            .iter()
            .position(|r| r.archetype == Archetype::ConstantMajority)
            .expect("constant_majority present");
        let ng_rank = rows[ng].rank;
        let cm_rank = rows[cm].rank;
        rows[ng].rank = cm_rank;
        rows[cm].rank = ng_rank;

        let lines = check_all(&rows);
        assert!(!lines[2].passed);
        assert!(lines[2].gaming_vector);
    }

    #[test]
    fn abstainer_tied_with_noisy_good_passes_invariant_6() {
        let mut rows = healthy_rows();
        let ng = rows
            .iter()
            .position(|r| r.archetype == Archetype::NoisyGood)
            .expect("noisy_good present");
        let ab = rows
            .iter()
            .position(|r| r.archetype == Archetype::Abstainer)
            .expect("abstainer present");
        rows[ab].rank = rows[ng].rank;
        let lines = check_all(&rows);
        assert!(lines[5].passed);
    }

    #[test]
    fn abstainer_above_noisy_good_fails_invariant_6() {
        let mut rows = healthy_rows();
        let ng = rows
            .iter()
            .position(|r| r.archetype == Archetype::NoisyGood)
            .expect("noisy_good present");
        let ab = rows
            .iter()
            .position(|r| r.archetype == Archetype::Abstainer)
            .expect("abstainer present");
        let ng_rank = rows[ng].rank;
        let ab_rank = rows[ab].rank;
        rows[ng].rank = ab_rank;
        rows[ab].rank = ng_rank;

        let lines = check_all(&rows);
        assert!(!lines[5].passed);
    }

    #[test]
    fn missing_archetype_fails_without_a_panic() {
        let rows: Vec<LeaderboardRow> = healthy_rows()
            .into_iter()
            .filter(|r| r.archetype != Archetype::Oracle)
            .collect();
        let lines = check_all(&rows);
        assert!(!lines[0].passed);
        assert!(lines[0].detail.contains("missing"));
    }

    #[test]
    fn render_marks_gaming_vector_lines() {
        let lines = check_all(&healthy_rows());
        let text = render(&lines, "VERDICT");
        assert!(text.contains("[GAMING VECTOR]"));
    }

    /// This is a `Standings` value where every invariant should pass
    /// under the `Eject` model: abstainer and malformer are ejected,
    /// with the right reason, and every other miner keeps a rank order
    /// that matches the healthy leaderboard.
    fn healthy_eject_standings() -> Standings {
        let ranked: Vec<LeaderboardRow> = healthy_rows()
            .into_iter()
            .filter(|r| r.archetype != Archetype::Abstainer && r.archetype != Archetype::Malformer)
            .enumerate()
            .map(|(i, mut r)| {
                r.rank = i + 1;
                r
            })
            .collect();
        Standings {
            model: AggregationModel::Eject,
            ranked,
            ejected: vec![
                EjectedRow {
                    archetype: Archetype::Abstainer,
                    first_failure_index: 3,
                    reason: EjectionReason::NoResponse,
                },
                EjectedRow {
                    archetype: Archetype::Malformer,
                    first_failure_index: 0,
                    reason: EjectionReason::MalformedResponse,
                },
            ],
        }
    }

    #[test]
    fn healthy_eject_standings_passes_every_invariant() {
        let lines = check_all_eject(&healthy_eject_standings());
        assert!(
            all_passed(&lines),
            "a line failed: {:?}",
            lines.iter().find(|l| !l.passed)
        );
        assert_eq!(lines.len(), 7);
    }

    #[test]
    fn eject_invariant_6_fails_when_abstainer_keeps_a_rank() {
        let mut standings = healthy_eject_standings();
        standings
            .ejected
            .retain(|r| r.archetype != Archetype::Abstainer);
        standings.ranked.push(row(Archetype::Abstainer, 99, 0.72));
        let lines = check_all_eject(&standings);
        assert!(!lines[5].passed);
    }

    #[test]
    fn eject_invariant_7_fails_on_the_wrong_reason() {
        let mut standings = healthy_eject_standings();
        for r in &mut standings.ejected {
            if r.archetype == Archetype::Malformer {
                r.reason = EjectionReason::NoResponse;
            }
        }
        let lines = check_all_eject(&standings);
        assert!(!lines[6].passed);
    }
}
