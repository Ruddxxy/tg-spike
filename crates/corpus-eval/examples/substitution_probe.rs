//! A bench for candidate text rules, run before any of them is shipped.
//!
//! Four benchmark rows invert in every band: the bad answer echoes the
//! ground truth's scaffolding and swaps the single word that carries
//! the meaning. It shares six of seven tokens, the correct bare answer
//! shares one, and the union divisor pays the echo.
//!
//! This example scores the TEXT BRANCH only, through `eval_script`'s own
//! public tokenizer, so a candidate rule can be measured in a second
//! instead of a wasm rebuild. A rule that wins here is then implemented
//! in `score.rs` and re-measured through the compiled module, which is
//! the only number that counts.
//!
//! ```text
//! cargo run --release -p corpus-eval --example substitution_probe
//! ```

use eval_script::text::{intersection_size, tokenize, TokenSet};

/// One measured case, with the direction it has to move.
struct Case {
    /// What the case is for.
    group: &'static str,
    /// The label printed in the table.
    name: &'static str,
    /// The ground truth text.
    truth: &'static str,
    /// The answer text.
    answer: &'static str,
    /// The bar this case has to clear, read by `verdict`.
    want: Want,
}

/// What a case has to do for a rule to survive it.
enum Want {
    /// This score must stay above the named case's score.
    Beats(&'static str),
    /// This score must not rise above the baseline's score for it.
    NoHigher,
    /// This score must stay above a floor, as a fraction of baseline.
    NoLowerThan(f64),
    /// This score is reported and judged by eye.
    Report,
}

/// The four rows that invert in every band, plus the row the label band
/// already fixed, plus the farms and the honest answers a rule must not
/// break.
fn cases() -> Vec<Case> {
    vec![
        // --- the four inversions ---------------------------------
        Case {
            group: "inversion",
            name: "q01 good",
            truth: "The scan verdict for this URL is malicious.",
            answer: "malicious",
            want: Want::Beats("q01 bad"),
        },
        Case {
            group: "inversion",
            name: "q01 bad",
            truth: "The scan verdict for this URL is malicious.",
            answer: "The scan verdict for this URL is clean.",
            want: Want::Report,
        },
        Case {
            group: "inversion",
            name: "q13 good",
            truth: "The sentiment of the review is positive.",
            answer: "positive",
            want: Want::Beats("q13 bad"),
        },
        Case {
            group: "inversion",
            name: "q13 bad",
            truth: "The sentiment of the review is positive.",
            answer: "The sentiment of the review is negative.",
            want: Want::Report,
        },
        Case {
            group: "inversion",
            name: "q21 good",
            truth: "The claim is false.",
            answer: "false",
            want: Want::Beats("q21 bad"),
        },
        Case {
            group: "inversion",
            name: "q21 bad",
            truth: "The claim is false.",
            answer: "The claim is true.",
            want: Want::Report,
        },
        Case {
            group: "inversion",
            name: "q24 good",
            truth: "Paris is the capital of France.",
            answer: "Paris",
            want: Want::Beats("q24 bad"),
        },
        Case {
            group: "inversion",
            name: "q24 bad",
            truth: "Paris is the capital of France.",
            answer: "Lyon is the capital of France.",
            want: Want::Report,
        },
        // --- the farms, which must not pay more ------------------
        Case {
            group: "farm",
            name: "one common token",
            truth: "is malicious",
            answer: "is",
            want: Want::NoHigher,
        },
        Case {
            group: "farm",
            name: "subset of truth",
            truth: "high risk malicious binary",
            answer: "malicious",
            want: Want::NoHigher,
        },
        Case {
            group: "farm",
            name: "scaffolding, no value",
            truth: "The temperature was 28.9 C.",
            answer: "the temperature was C",
            want: Want::NoHigher,
        },
        Case {
            group: "farm",
            name: "scaffolding, one word",
            truth: "The temperature was 28.9 C.",
            answer: "temperature",
            want: Want::NoHigher,
        },
        Case {
            group: "farm",
            name: "unit only",
            truth: "192.43 USD",
            answer: "USD",
            want: Want::NoHigher,
        },
        Case {
            group: "farm",
            name: "json key echo",
            truth: "{\"summary\":\"28.9 C in Paris\"}",
            answer: "summary",
            want: Want::NoHigher,
        },
        Case {
            group: "farm",
            name: "padding, repeated",
            truth: "malicious",
            answer: "malicious filler filler filler filler",
            want: Want::NoHigher,
        },
        Case {
            group: "farm",
            name: "padding, distinct",
            truth: "malicious",
            answer: "malicious f1 f2 f3 f4 f5 f6 f7 f8",
            want: Want::NoHigher,
        },
        // --- several foreign tokens, which is what tells R3, R4 and
        //     R5 apart. Every case above has exactly one, so all three
        //     score them the same and none of them is chosen yet.
        Case {
            group: "multi-foreign",
            name: "swap + reword, bad",
            truth: "Paris is the capital of France.",
            answer: "Lyon is the biggest capital of France",
            want: Want::Report,
        },
        Case {
            group: "multi-foreign",
            name: "swap + pad, bad",
            truth: "The claim is false.",
            answer: "The claim is true beyond any doubt at all",
            want: Want::Report,
        },
        // --- honest answers, which must not be crushed -----------
        Case {
            group: "honest",
            name: "self match",
            truth: "Paris is the capital of France.",
            answer: "Paris is the capital of France.",
            want: Want::NoLowerThan(1.0),
        },
        Case {
            group: "honest",
            name: "elaborated",
            truth: "malicious",
            answer: "definitely malicious",
            want: Want::NoLowerThan(1.0),
        },
        Case {
            group: "honest",
            name: "reworded, right",
            truth: "Paris is the capital of France.",
            answer: "The capital is Paris",
            want: Want::NoLowerThan(0.5),
        },
        Case {
            group: "honest",
            name: "short + one foreign",
            truth: "The scan verdict for this URL is malicious.",
            answer: "It is malicious",
            want: Want::NoLowerThan(0.5),
        },
        Case {
            group: "honest",
            name: "partial, right",
            truth: "The sentiment of the review is positive.",
            answer: "The sentiment is positive",
            want: Want::NoLowerThan(1.0),
        },
        // Correct, and phrased with several words the truth never used.
        // This is the case a harsh per-foreign-token charge destroys.
        Case {
            group: "honest",
            name: "right, own words",
            truth: "The sentiment of the review is positive.",
            answer: "I think it is positive overall",
            want: Want::NoLowerThan(0.5),
        },
        Case {
            group: "honest",
            name: "right, hedged",
            truth: "The claim is false.",
            answer: "As far as I can tell the claim is false",
            want: Want::NoLowerThan(1.0),
        },
        Case {
            group: "honest",
            name: "right, verbose",
            truth: "Paris is the capital of France.",
            answer: "Paris has been the capital city of France since 987",
            want: Want::NoLowerThan(0.5),
        },
    ]
}

/// The pieces every candidate rule reads.
struct Parts {
    /// Tokens both sides hold.
    shared: usize,
    /// Tokens the ground truth holds.
    truth_len: usize,
    /// Tokens the answer holds.
    answer_len: usize,
}

impl Parts {
    /// Tokens the answer asserts that the truth does not hold.
    fn foreign(&self) -> usize {
        self.answer_len - self.shared
    }
    /// Tokens the truth holds that the answer leaves out.
    fn missing(&self) -> usize {
        self.truth_len - self.shared
    }
    /// The fraction of the truth the answer gives back.
    fn recall(&self) -> f64 {
        if self.truth_len == 0 {
            return 0.0;
        }
        (self.shared as f64) / (self.truth_len as f64)
    }
    /// The shipped rule: shared over union.
    fn jaccard(&self) -> f64 {
        let union = self.truth_len + self.answer_len - self.shared;
        if union == 0 {
            return 0.0;
        }
        (self.shared as f64) / (union as f64)
    }
}

/// This function reads the two texts into the counts a rule needs.
fn parts(truth: &str, answer: &str) -> Parts {
    let truth_tokens: TokenSet = tokenize(truth);
    let answer_tokens: TokenSet = tokenize(answer);
    Parts {
        shared: intersection_size(&truth_tokens, &answer_tokens),
        truth_len: truth_tokens.len(),
        answer_len: answer_tokens.len(),
    }
}

/// One candidate rule: a name and the score it gives.
struct Rule {
    /// The name printed in the table.
    name: &'static str,
    /// What the rule does, in one line.
    idea: &'static str,
    /// The scoring function.
    score: fn(&Parts) -> f64,
}

/// R0. The shipped rule, for the baseline column.
fn r0_baseline(p: &Parts) -> f64 {
    p.jaccard()
}

/// R1. An answer that asserts nothing foreign is scored on precision.
fn r1_precision_if_subset(p: &Parts) -> f64 {
    if p.foreign() == 0 && p.answer_len > 0 {
        return 1.0;
    }
    p.jaccard()
}

/// R2. Every foreign token costs a flat factor.
fn r2_flat_foreign(p: &Parts) -> f64 {
    p.jaccard() / 8f64.powi(p.foreign() as i32)
}

/// R3. THE SUBSTITUTION RULE. A token the answer leaves out is cheap. A
/// token the answer asserts INSTEAD is expensive, and it is the more
/// expensive the more of the truth the answer echoed around it.
fn r3_substitution(p: &Parts) -> f64 {
    if p.foreign() >= 1 && p.missing() >= 1 {
        return p.jaccard() * (1.0 - p.recall()).powi(p.foreign() as i32);
    }
    p.jaccard()
}

/// R4. The same, but only the substituted pairs are charged, so a long
/// tail of extra words costs no more than the swap itself.
fn r4_substitution_paired(p: &Parts) -> f64 {
    let pairs = p.foreign().min(p.missing());
    if pairs >= 1 {
        return p.jaccard() * (1.0 - p.recall()).powi(pairs as i32);
    }
    p.jaccard()
}

/// R5. The same as R3 with the charge applied once, whatever the count.
fn r5_substitution_once(p: &Parts) -> f64 {
    if p.foreign() >= 1 && p.missing() >= 1 {
        return p.jaccard() * (1.0 - p.recall());
    }
    p.jaccard()
}

/// R6. Strength comes from how TIGHT the echo is, not from recall.
///
/// A substitution reproduces the truth almost exactly and changes one
/// thing, so it scores high on the shipped rule. A correct answer in
/// the miner's own words covers the truth loosely and scores lower. So
/// the charge rides on the shipped score itself: the closer an answer
/// is to the truth without being it, the more a foreign token reads as
/// a swap rather than as phrasing.
fn r6_tightness(p: &Parts) -> f64 {
    let j = p.jaccard();
    if p.foreign() >= 1 && p.missing() >= 1 {
        return j * (1.0 - j) * (1.0 - j);
    }
    j
}

/// R7. The same charge, but only once the echo is tight enough to read
/// as one. Below half, an answer is phrased differently rather than
/// copied, and it is left alone.
fn r7_tightness_gated(p: &Parts) -> f64 {
    let j = p.jaccard();
    if p.foreign() >= 1 && p.missing() >= 1 && j >= 0.5 {
        return j * (1.0 - j) * (1.0 - j);
    }
    j
}

fn rules() -> Vec<Rule> {
    vec![
        Rule {
            name: "R0 shipped",
            idea: "shared / union",
            score: r0_baseline,
        },
        Rule {
            name: "R1 subset=1",
            idea: "no foreign token -> 1.0",
            score: r1_precision_if_subset,
        },
        Rule {
            name: "R2 flat/8^f",
            idea: "each foreign token costs 8x",
            score: r2_flat_foreign,
        },
        Rule {
            name: "R3 subst",
            idea: "omit is cheap, substitute costs (1-recall)^f",
            score: r3_substitution,
        },
        Rule {
            name: "R4 subst pair",
            idea: "charge only min(missing, foreign) swaps",
            score: r4_substitution_paired,
        },
        Rule {
            name: "R5 subst once",
            idea: "charge one swap, whatever the count",
            score: r5_substitution_once,
        },
        Rule {
            name: "R6 tight",
            idea: "charge (1-j)^2, strength from echo tightness",
            score: r6_tightness,
        },
        Rule {
            name: "R7 tight>=.5",
            idea: "R6, only once the echo is at least half",
            score: r7_tightness_gated,
        },
    ]
}

fn main() {
    let cases = cases();
    let rules = rules();

    let scores: Vec<Vec<f64>> = rules
        .iter()
        .map(|rule| {
            cases
                .iter()
                .map(|case| (rule.score)(&parts(case.truth, case.answer)))
                .collect()
        })
        .collect();

    println!("TEXT BRANCH ONLY. The dispatch, negation and copy-question");
    println!("rules run before this and are not changed by any candidate.\n");

    print!("{:<24} {:<22}", "case", "group");
    for rule in &rules {
        print!(" {:>13}", rule.name);
    }
    println!();
    println!("{}", "-".repeat(46 + 14 * rules.len()));

    for (index, case) in cases.iter().enumerate() {
        print!("{:<24} {:<22}", case.name, case.group);
        for row in &scores {
            print!(" {:>13.4}", row[index]);
        }
        println!();
    }

    println!("\n\nVERDICT PER RULE");
    println!("{}", "=".repeat(78));
    for (rule_index, rule) in rules.iter().enumerate() {
        let row = &scores[rule_index];
        let base = &scores[0];
        let mut broken: Vec<String> = Vec::new();
        let mut fixed = 0usize;

        for (index, case) in cases.iter().enumerate() {
            match case.want {
                Want::Beats(other) => {
                    let other_index = cases.iter().position(|c| c.name == other).unwrap();
                    if row[index] > row[other_index] {
                        fixed += 1;
                    } else {
                        broken.push(format!("{} still loses to {other}", case.name));
                    }
                }
                Want::NoHigher => {
                    // A farm must never pay more than it pays today.
                    if row[index] > base[index] + 1e-12 {
                        broken.push(format!(
                            "FARM REOPENED {}: {:.4} -> {:.4}",
                            case.name, base[index], row[index]
                        ));
                    }
                }
                Want::NoLowerThan(fraction) => {
                    let floor = base[index] * fraction - 1e-12;
                    if row[index] < floor {
                        broken.push(format!(
                            "honest answer crushed {}: {:.4} -> {:.4}",
                            case.name, base[index], row[index]
                        ));
                    }
                }
                Want::Report => {}
            }
        }

        println!("\n{}  ({})", rule.name, rule.idea);
        println!("   inversions fixed   {fixed}/4");
        if broken.is_empty() {
            println!("   nothing broken");
        } else {
            for line in &broken {
                println!("   {line}");
            }
        }
    }
}
