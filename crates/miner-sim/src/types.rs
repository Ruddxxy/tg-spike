//! This module holds the data types that the other modules share.
//!
//! This module holds data only. It holds no generation logic and no
//! scoring logic. `dataset.rs` and `archetype.rs` make these types.
//! `scoring.rs`, `leaderboard.rs`, and `bootstrap.rs` read these types.
//!
//! The types in this module are a contract between the modules. Do not
//! change a field name or a variant name without a change to every
//! module that reads it.
//!
//! # Score direction
//!
//! A HIGH score is good. A LOW score is bad. The range is 0.0 to 1.0.
//! The best score is 1.0. The worst score is 0.0. The source is the
//! Telegraph whitepaper v1.0, section 7.4 and section 4.3.
//!
//! # The two layers
//!
//! The simulator keeps two layers apart. Do not mix them.
//!
//! - Layer 1 is the SCRIPT (`eval-script`). It gives one score in the
//!   range 0.0 to 1.0 for one item. It has no memory between items.
//! - Layer 2 is the PROTOCOL (this crate). It collects the item scores.
//!   It then applies the ejection rule. The script does not know about
//!   the ejection rule.

/// One item of the labelled data set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Item {
    /// The position of the item in the data set. The first item has
    /// index 0.
    pub index: usize,
    /// The correct answer. The value is 0 or 1.
    pub label: u8,
    /// The latent signal strength of the item. The range is 0.0 to 1.0.
    ///
    /// A high value means an easy item. A miner reads a clear signal and
    /// gives the correct answer. A low value means a hard item. A value
    /// near 0.0 means the item is almost impossible. No miner can do
    /// better than a guess on such an item.
    pub signal: f64,
}

impl Item {
    /// This function makes the ground truth JSON for the item.
    ///
    /// The output is the exact text that a validator sends to the
    /// script. An example is `{"label": 1}`.
    #[must_use]
    pub fn ground_truth_json(&self) -> String {
        format!("{{\"label\": {}}}", self.label)
    }
}

/// A seed for the miner response stream.
///
/// # Why this type exists
///
/// The data set generator and the miner response generator use the same
/// PRNG algorithm. The generator draws 2 values for each item. The
/// calibrated miner core also draws 2 values for each item. If the two
/// seeds hold the same value, the miner correctness draw lands on the
/// exact random value that made the signal of that same item. The
/// correctness of the miner then becomes a function of the difficulty of
/// the item. The miner stops being calibrated. Every test still passes,
/// and the leaderboard still looks correct. This defect occurred once
/// already in this crate.
///
/// This type stops that defect at compile time. `Dataset` cannot make a
/// `ResponseSeed`. There is no `From<u64>` for this type. A caller must
/// use [`ResponseSeed::derive`], which applies a fixed mask, or
/// [`ResponseSeed::new_unchecked`], whose name shows the risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResponseSeed(u64);

impl ResponseSeed {
    /// The mask that keeps the response stream away from the data set
    /// stream.
    const MASK: u64 = 0xA5A5_A5A5_A5A5_A5A5;

    /// This function makes a response seed from a data set seed.
    ///
    /// The function applies a fixed mask. The result never has the same
    /// value as the input. Use this function.
    #[must_use]
    pub fn derive(dataset_seed: u64) -> Self {
        ResponseSeed(dataset_seed ^ Self::MASK)
    }

    /// This function makes a response seed from a raw value.
    ///
    /// The caller must make sure the value is not the seed of the data
    /// set. Read the note on [`ResponseSeed`]. Use
    /// [`ResponseSeed::derive`] if you can.
    #[must_use]
    pub fn new_unchecked(value: u64) -> Self {
        ResponseSeed(value)
    }

    /// This function gives the raw seed value for the PRNG.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

/// The shape of a data set.
///
/// The shape controls the base rate and the signal strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DatasetShape {
    /// The base rate is 0.50. The signal strength is uniform.
    Balanced,
    /// The base rate is 0.90. The signal strength is uniform.
    Skewed,
    /// The base rate is 0.50. 20 percent of the items are almost
    /// impossible.
    HardTail,
}

impl DatasetShape {
    /// Every data set shape, in a fixed order.
    pub const ALL: [DatasetShape; 3] = [
        DatasetShape::Balanced,
        DatasetShape::Skewed,
        DatasetShape::HardTail,
    ];

    /// This function gives the short name of the shape.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            DatasetShape::Balanced => "balanced",
            DatasetShape::Skewed => "skewed",
            DatasetShape::HardTail => "hard_tail",
        }
    }

    /// This function gives the target base rate of the shape.
    ///
    /// The base rate is the fraction of items with label 1. The realised
    /// value of a data set differs a little from the target. Read
    /// [`Dataset::realised_base_rate`].
    #[must_use]
    pub fn base_rate(self) -> f64 {
        match self {
            DatasetShape::Balanced | DatasetShape::HardTail => 0.50,
            DatasetShape::Skewed => 0.90,
        }
    }
}

/// A labelled data set and the facts that the miners need about it.
#[derive(Debug, Clone)]
pub struct Dataset {
    /// The shape that made this data set.
    pub shape: DatasetShape,
    /// The seed that made this data set.
    ///
    /// Do not use this value as a response seed. Read the note on
    /// [`ResponseSeed`].
    pub seed: u64,
    /// The items, in generation order.
    pub items: Vec<Item>,
    /// The label that occurs most often in this data set.
    ///
    /// The `constant_majority` miner always gives this label. A tie
    /// gives label 1.
    pub majority_label: u8,
    /// The measured fraction of items with label 1.
    ///
    /// The `bayes_calibrated_good` miner uses this value as its prior.
    /// The value stands for the class balance that a real miner reads
    /// from the history of an intent.
    pub realised_base_rate: f64,
    /// The signal value at the 30th percentile of this data set.
    ///
    /// The `abstainer` miner does not answer an item with a signal
    /// value at or below this threshold. Those items are the hardest
    /// 30 percent.
    pub hard_signal_threshold: f64,
}

/// The class of a miner response.
///
/// The simulator counts the classes for the leaderboard. The class does
/// not change the score. The WASM module scores the response text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResponseKind {
    /// The miner gave a well formed answer.
    Answer {
        /// The confidence that the miner reported. The range is 0.0 to
        /// 1.0. The value is the probability of label 1.
        confidence: f64,
        /// True if the answer agrees with the label of the item.
        ///
        /// The simulator counts an answer as correct when the reported
        /// confidence falls on the correct side of 0.5.
        correct: bool,
    },
    /// The miner did not answer. The response text is `{}`.
    Abstain,
    /// The miner sent text that the script cannot read.
    Malformed,
}

/// One response from one miner for one item.
#[derive(Debug, Clone)]
pub struct Response {
    /// The exact text that the miner sends over the wire.
    ///
    /// The simulator sends these bytes to the WASM module without a
    /// change. A malformed response holds malformed text on purpose.
    pub json: String,
    /// The class of the response.
    pub kind: ResponseKind,
}

/// A type of miner.
///
/// Each archetype has a known true quality. The simulator builds the
/// quality in. The leaderboard must then show that quality back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Archetype {
    /// This miner is always correct. It reports confidence 0.99.
    Oracle,
    /// This miner has a target accuracy of 0.85. Its confidence is
    /// calibrated against the item signal.
    ///
    /// The target applies to an item with a signal of 1.0. The core
    /// damps the target by the item signal. Thus the mean accuracy on
    /// a data set with a uniform signal is near 0.675, not 0.85.
    ///
    /// This miner does not know the class base rate. It is calibrated
    /// against the signal only. On a data set with a base rate that is
    /// not 0.5, it is NOT calibrated against the joint distribution.
    /// Read [`Archetype::BayesCalibratedGood`].
    NoisyGood,
    /// This miner has a target accuracy of 0.65. Its confidence is
    /// calibrated against the item signal.
    ///
    /// The signal damps the target in the same way as `NoisyGood`. The
    /// mean accuracy on a uniform signal data set is near 0.575.
    NoisyMediocre,
    /// This miner always gives the majority class with confidence 0.99.
    ConstantMajority,
    /// This miner reports a uniform random confidence.
    Random,
    /// This miner has the accuracy of `NoisyGood`. It pushes its
    /// confidence toward 0.0 or 1.0, so it is not calibrated.
    OverconfidentGood,
    /// This miner has the accuracy of `NoisyGood`. It pulls its
    /// confidence toward 0.5, so it is not calibrated.
    UnderconfidentGood,
    /// This miner is correct when it answers. It sends `{}` for the
    /// hardest 30 percent of the items.
    Abstainer,
    /// This miner is `NoisyGood`, but 10 percent of its responses are
    /// malformed.
    Malformer,
    /// This miner inverts the answer of `NoisyGood`.
    ///
    /// The mean accuracy is near 0.325, which is `1.0 - 0.675`. It is
    /// not 0.15. The signal damping sets the value. Read the note on
    /// `NoisyGood`.
    Contrarian,
    /// This miner has the signal accuracy of `NoisyGood`. It also
    /// applies the class base rate as a prior.
    ///
    /// The miner reads the class balance from the history of the
    /// intent. A real miner can do this. The miner then moves its
    /// signal-conditional confidence `c` to the true posterior:
    ///
    /// ```text
    /// posterior = (c * B) / (c * B + (1 - c) * (1 - B))
    /// ```
    ///
    /// `B` is the realised base rate of the data set. On a data set
    /// with a base rate of 0.5, the posterior equals `c`, so this miner
    /// gives the same answers as `NoisyGood`. On a data set with a
    /// skewed base rate, this miner is calibrated against the joint
    /// distribution and `NoisyGood` is not.
    BayesCalibratedGood,
}

impl Archetype {
    /// Every archetype, in a fixed order.
    pub const ALL: [Archetype; 11] = [
        Archetype::Oracle,
        Archetype::NoisyGood,
        Archetype::NoisyMediocre,
        Archetype::ConstantMajority,
        Archetype::Random,
        Archetype::OverconfidentGood,
        Archetype::UnderconfidentGood,
        Archetype::Abstainer,
        Archetype::Malformer,
        Archetype::Contrarian,
        Archetype::BayesCalibratedGood,
    ];

    /// This function gives the short name of the archetype.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Archetype::Oracle => "oracle",
            Archetype::NoisyGood => "noisy_good",
            Archetype::NoisyMediocre => "noisy_mediocre",
            Archetype::ConstantMajority => "constant_majority",
            Archetype::Random => "random",
            Archetype::OverconfidentGood => "overconfident_good",
            Archetype::UnderconfidentGood => "underconfident_good",
            Archetype::Abstainer => "abstainer",
            Archetype::Malformer => "malformer",
            Archetype::Contrarian => "contrarian",
            Archetype::BayesCalibratedGood => "bayes_calibrated_good",
        }
    }

    /// This function tells if the archetype reports a calibrated
    /// confidence.
    ///
    /// A calibrated miner is correct about `c` of the time when it
    /// reports confidence `c`. The calibration test checks only these
    /// archetypes.
    ///
    /// `NoisyGood` and `NoisyMediocre` are calibrated only when the base
    /// rate is 0.5. `BayesCalibratedGood` is calibrated at every base
    /// rate.
    ///
    /// The `random` miner is not in this list. It reports a uniform
    /// confidence that carries no information about the label. Thus the
    /// share of label 1 items at a reported confidence `c` stays at the
    /// base rate. It does not follow `c`.
    #[must_use]
    pub fn is_calibrated(self) -> bool {
        matches!(
            self,
            Archetype::NoisyGood | Archetype::NoisyMediocre | Archetype::BayesCalibratedGood
        )
    }
}

/// A scoring rule that the WASM module gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Metric {
    /// The converted Brier score. The export name is `score`.
    ///
    /// The module returns `1.0 - brier`. A high value is good.
    ///
    /// The Brier rule is proper. A miner gets its best score when it
    /// reports its true belief.
    Brier,
    /// The converted normalised log loss. The export name is
    /// `score_log_loss`.
    ///
    /// The module returns `1.0 - normalised_log_loss`. A high value is
    /// good.
    LogLoss,
}

impl Metric {
    /// Every metric, in a fixed order.
    pub const ALL: [Metric; 2] = [Metric::Brier, Metric::LogLoss];

    /// This function gives the short name of the metric.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Metric::Brier => "brier (score)",
            Metric::LogLoss => "log_loss (score_log_loss)",
        }
    }
}

/// The reason that the protocol removed a miner from the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EjectionReason {
    /// The miner did not answer the query.
    NoResponse,
    /// The miner sent a response that the script cannot read.
    MalformedResponse,
}

impl EjectionReason {
    /// This function gives the short name of the reason.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            EjectionReason::NoResponse => "no_response",
            EjectionReason::MalformedResponse => "malformed_response",
        }
    }
}

/// The first failure of a miner in one epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Failure {
    /// The item index where the failure occurred.
    pub index: usize,
    /// The reason for the failure.
    pub reason: EjectionReason,
}

/// The state of a miner at the end of one epoch.
///
/// This type belongs to Layer 2, the protocol. The script does not know
/// about this type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MinerStatus {
    /// The miner stays in the routing pool.
    Ranked {
        /// The mean of the item scores. A high value is good.
        mean_score: f64,
        /// The median of the item scores. A high value is good.
        median: f64,
    },
    /// The protocol removed the miner from the routing pool.
    ///
    /// An ejected miner does not get a score of 0.0 and last place. The
    /// protocol removes it from the pool. Those two rules are not the
    /// same.
    Ejected {
        /// The item index of the first failure.
        first_failure_index: usize,
        /// The reason for the ejection.
        reason: EjectionReason,
    },
}

/// The way that the protocol turns item scores into one result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AggregationModel {
    /// Every miner keeps its place. A failure gets the worst item score
    /// of 0.0, and the mean holds that value.
    ///
    /// This model is NOT the protocol rule. The simulator keeps it for
    /// the comparison. It shows how much the aggregation layer changes
    /// the outcome.
    ScoreAndKeep,
    /// The first failure removes the miner from the pool for the epoch.
    ///
    /// This model is the protocol rule. The source is the whitepaper
    /// v1.0, section 5.1.
    Eject,
}

impl AggregationModel {
    /// Every aggregation model, in a fixed order.
    pub const ALL: [AggregationModel; 2] =
        [AggregationModel::ScoreAndKeep, AggregationModel::Eject];

    /// This function gives the short name of the model.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            AggregationModel::ScoreAndKeep => "score_and_keep",
            AggregationModel::Eject => "eject",
        }
    }
}

/// The scores of one miner on one data set with one metric.
#[derive(Debug, Clone)]
pub struct MinerResult {
    /// The archetype that made the responses.
    pub archetype: Archetype,
    /// The score of each item, in item order.
    ///
    /// The WASM module made every value. Every value is in the range
    /// 0.0 to 1.0. A HIGH value is good.
    pub scores: Vec<f64>,
    /// The count of malformed responses.
    pub n_malformed: usize,
    /// The count of responses that did not answer.
    pub n_abstained: usize,
    /// The first response that would eject the miner.
    ///
    /// The value is `None` if the miner answered every item. The
    /// `Eject` aggregation model reads this field. The `ScoreAndKeep`
    /// model does not read it.
    pub first_failure: Option<Failure>,
}

/// One row of a leaderboard.
#[derive(Debug, Clone)]
pub struct LeaderboardRow {
    /// The position of the miner. The best miner has rank 1.
    pub rank: usize,
    /// The archetype of the miner.
    pub archetype: Archetype,
    /// The mean of the item scores. A HIGH value is good.
    pub mean_score: f64,
    /// The median of the item scores. A HIGH value is good.
    pub median_score: f64,
    /// The count of malformed responses.
    pub n_malformed: usize,
    /// The count of responses that did not answer.
    pub n_abstained: usize,
}

/// One row of the ejected miner list.
#[derive(Debug, Clone)]
pub struct EjectedRow {
    /// The archetype of the miner.
    pub archetype: Archetype,
    /// The item index of the first failure.
    pub first_failure_index: usize,
    /// The reason for the ejection.
    pub reason: EjectionReason,
}

/// The full result of one aggregation model on one data set.
#[derive(Debug, Clone)]
pub struct Standings {
    /// The model that made these standings.
    pub model: AggregationModel,
    /// The miners that stay in the pool, in rank order.
    pub ranked: Vec<LeaderboardRow>,
    /// The miners that the protocol removed from the pool.
    ///
    /// This list is always empty for the `ScoreAndKeep` model.
    pub ejected: Vec<EjectedRow>,
}

/// One row of the Brier Skill Score table.
#[derive(Debug, Clone)]
pub struct SkillRow {
    /// The archetype of the miner.
    pub archetype: Archetype,
    /// The mean RAW Brier score of the miner. A LOW value is good.
    ///
    /// This value is `1.0 - converted_score`. The skill score needs the
    /// raw loss, not the converted score.
    pub raw_brier: f64,
    /// The Brier score of a forecaster that knows only the base rate.
    ///
    /// The value is `base_rate * (1.0 - base_rate)`.
    pub climatology_brier: f64,
    /// The Brier Skill Score. A value above 0.0 shows real skill.
    ///
    /// The value is `1.0 - raw_brier / climatology_brier`. A value below
    /// 0.0 means the miner is worse than a forecaster that knows only
    /// the base rate.
    pub bss: f64,
}

/// One line of an invariant check report.
///
/// `verdict.rs` builds one of these values for each order invariant
/// that it checks. `lib.rs` re-exports this type from this module, so
/// this type must live here and not in `verdict.rs`.
///
/// # Note on this addition
///
/// `lib.rs` already re-exports `types::VerdictLine`, and `verdict.rs`
/// already imports `crate::types::VerdictLine`, but no earlier version
/// of this file defined the type. Without this struct the crate could
/// not compile at all, for any change in any file. This addition adds
/// one new, self-contained struct. It does not change a field or a
/// variant of any other type in this file.
#[derive(Debug, Clone)]
pub struct VerdictLine {
    /// The number of the invariant, starting at 1.
    pub number: usize,
    /// The fixed statement of the invariant.
    pub statement: &'static str,
    /// True when the leaderboard keeps the invariant.
    pub passed: bool,
    /// The rank and score numbers behind the check, whether the check
    /// passed or failed.
    pub detail: String,
    /// True when a miner could use this rule to game its rank without
    /// a real gain in quality.
    pub gaming_vector: bool,
}
