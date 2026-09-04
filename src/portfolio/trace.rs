//! What a portfolio ran, candidate by candidate.
//!
//! The portfolio keeps the narrowest decomposition and says nothing about
//! where it came from. A trace sink is told about every candidate as it
//! finishes, so a caller can attribute a result to the candidate that produced
//! it.

use std::fmt;
use std::time::Duration;

/// Which candidate of the schedule this was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stage {
    /// Min-fill, deterministic or sampled.
    MinFill,
    /// Min-degree, deterministic or sampled.
    MinDegree,
    /// Nested dissection.
    NestedDissection,
    /// One of the diverse fill-degree candidates.
    Diverse {
        /// The candidate's degree coefficient.
        degree_coefficient: i8,
    },
    /// An ordinary sampled min-fill restart.
    Sample,
    /// The trailing FlowCutter candidate.
    FlowCutter,
    /// A hedge's weighted stage as a whole, rather than one of its candidates.
    WeightedStage,
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stage::MinFill => formatter.write_str("min-fill"),
            Stage::MinDegree => formatter.write_str("min-degree"),
            Stage::NestedDissection => formatter.write_str("nested-dissection"),
            Stage::Diverse { degree_coefficient } => {
                write!(formatter, "diverse:{degree_coefficient}")
            }
            Stage::Sample => formatter.write_str("sample"),
            Stage::FlowCutter => formatter.write_str("flowcutter"),
            Stage::WeightedStage => formatter.write_str("weighted-stage"),
        }
    }
}

/// Which pass of a hedged schedule a candidate belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pass {
    /// Nothing is hedged, so there is one pass and this is it.
    #[default]
    Only,
    /// The portfolio's own candidate, run on the caller's weights.
    Plain,
    /// The same candidate on the weights of one of the hedge's stages.
    Modified {
        /// Which weighted stage it belongs to, counting from zero in the order
        /// the hedge's weightings run. A hedge of one weighting only ever
        /// reports 0.
        index: u8,
    },
}

/// What one candidate left behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateOutcome {
    /// A decomposition, recorded and folded into the best width so far.
    Produced {
        /// The decomposition's width.
        width: u32,
        /// The total size of its bags.
        total_bag_size: usize,
        /// Whether the portfolio would now return this one.
        best: bool,
    },
    /// A bag passed the width bound, so nothing usable came back. That bound
    /// comes from a candidate that already produced one, so a winner exists.
    WidthAborted,
    /// The hard deadline was reached, with or without a completed residual.
    /// No later candidate starts.
    DeadlineReached,
    /// A candidate the portfolio did not start, and so has no result for. The
    /// nested-dissection slot reports this on a residual in the middle band
    /// (see [`PortfolioConfig`](crate::portfolio::PortfolioConfig)): it reads
    /// its deadline between levels, and at that size one level can run past the
    /// portfolio's hard deadline.
    NotStarted,
    /// A weighted stage the budget rule did not start: what one more stage was
    /// projected to cost did not fit in what the stages may spend, so the
    /// restarts keep the time. Reported once per stage left unrun, against
    /// [`Stage::WeightedStage`].
    StageSkipped {
        /// What one more stage was projected to cost.
        projected: Duration,
        /// What the stages that ran have spent between them.
        spent: Duration,
        /// What the stages may spend between them: the reserve fraction of what
        /// the soft budget had left when the plain pass finished.
        allowance: Duration,
    },
}

/// One candidate the portfolio ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidateTrace {
    /// Which candidate of the schedule this was.
    pub stage: Stage,
    /// The seed it ran on.
    pub seed: u64,
    /// Which pass of a hedged schedule it belongs to.
    pub pass: Pass,
    /// What it left behind.
    pub outcome: CandidateOutcome,
    /// How far into the portfolio it finished, on the clock the deadlines use:
    /// wall time normally, and charged construction work while the meter is
    /// armed.
    pub elapsed: Duration,
}
