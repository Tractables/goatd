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
    /// The maximum cardinality search candidate.
    MaximumCardinality,
    /// The MCS-M candidate, which eliminates along a minimal triangulation of
    /// the residual.
    MinimalTriangulation,
    /// The pass that drops the fill edges the winner's bags do not need.
    Minimalized,
    /// The stage that recombines the bags of all the candidates.
    Recombined,
    /// The stage that merges independently built decompositions into the best
    /// one the run has.
    Merged,
    /// The trailing FlowCutter candidate.
    FlowCutter,
    /// The lift of a decomposition of one side's projection, on a bipartite
    /// graph.
    BipartiteLift,
    /// A hedge's weighted stage as a whole, rather than one of its candidates.
    WeightedStage,
    /// The ordinary sampled restarts as a whole, rather than one of them.
    SampledRestarts,
}

impl Stage {
    /// Which slot of the recombination pool this stage's decompositions go in.
    ///
    /// The pool keeps the best decomposition of each slot, so every stage that
    /// produced one is represented in the bags the recombination reads however
    /// wide it came out: what the search needs from a candidate is a tree
    /// shaped differently from the others, and the stages are what differ.
    pub(crate) fn slot(self) -> u32 {
        match self {
            Stage::MinFill => 0,
            Stage::MinDegree => 1,
            Stage::NestedDissection => 2,
            Stage::Sample => 3,
            Stage::MaximumCardinality => 4,
            Stage::MinimalTriangulation => 5,
            Stage::Minimalized => 6,
            Stage::Recombined => 7,
            Stage::Merged => 12,
            Stage::FlowCutter => 8,
            Stage::WeightedStage => 9,
            Stage::SampledRestarts => 10,
            Stage::BipartiteLift => 11,
            Stage::Diverse { degree_coefficient } => {
                100 + u32::from(degree_coefficient.cast_unsigned())
            }
        }
    }
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
            Stage::MaximumCardinality => formatter.write_str("maximum-cardinality"),
            Stage::BipartiteLift => formatter.write_str("bipartite-lift"),
            Stage::MinimalTriangulation => formatter.write_str("minimal-triangulation"),
            Stage::Minimalized => formatter.write_str("minimalized"),
            Stage::Recombined => formatter.write_str("recombined"),
            Stage::Merged => formatter.write_str("merged"),
            Stage::FlowCutter => formatter.write_str("flowcutter"),
            Stage::WeightedStage => formatter.write_str("weighted-stage"),
            Stage::SampledRestarts => formatter.write_str("sampled-restarts"),
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

/// Two numbers about a produced decomposition's shape, beside its width and
/// total bag size, for a caller that ranks the candidates itself.
///
/// Computed only on a traced run: [`decompose`](crate::portfolio::decompose)
/// and [`candidates`](crate::portfolio::candidates) do not pay for them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    /// `log2` of the sum over bags of `2^|bag|`: what a consumer compiling
    /// over the bags pays in the worst case. Equal widths can differ here by
    /// the number of bags at that width and the sizes of the rest.
    pub bag_mass: f64,
    /// The largest number of vertices two adjacent bags share: what a consumer
    /// carries across the widest join in the tree.
    pub max_separator: usize,
}

/// What one candidate left behind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CandidateOutcome {
    /// A decomposition, recorded and folded into the best width so far.
    Produced {
        /// The decomposition's width.
        width: u32,
        /// The total size of its bags.
        total_bag_size: usize,
        /// Its bag mass and widest separator, on a traced run; `None` where
        /// the run has no sink to report them to and does not compute them,
        /// and for a candidate wider than the incumbent on a run that keeps
        /// only its best, which is dropped before they are computed.
        shape: Option<Shape>,
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
    /// The ordinary restarts stopped because they had stalled: the last one to
    /// improve the portfolio's best decomposition was far enough back that the
    /// patience rule gave up on the rest of the list. Reported once, against
    /// [`Stage::SampledRestarts`].
    SamplingStopped {
        /// Ordinary restarts that had run.
        restarts: u64,
        /// The last of them to improve the best decomposition, counting from
        /// zero, or `None` where none of them did.
        last_improvement: Option<u64>,
        /// What was left of the deadline the restarts run against, or `None`
        /// where they run under no deadline.
        left: Option<Duration>,
    },
    /// The trailing FlowCutter candidate ran under a patience: what it had,
    /// how long it was allowed to go without improving, and what it spent.
    /// Reported once beside that candidate's own record, and only where the
    /// caller turned the patience rule on: the fixed patience a short window
    /// has always had is not this rule's doing and gets no record. The backend reports no reason for stopping,
    /// so a run that ended well inside its window is one the patience ended.
    TailBounded {
        /// The window the candidate was given.
        window: Duration,
        /// How long it could go without a narrower decomposition.
        patience: Duration,
        /// What it spent.
        spent: Duration,
    },
}

/// Which candidate of the schedule a decomposition came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidateOrigin {
    /// Which candidate of the schedule this was.
    pub stage: Stage,
    /// The seed it ran on.
    pub seed: u64,
    /// Which pass of a hedged schedule it belongs to.
    pub pass: Pass,
}

/// One candidate the portfolio ran.
#[derive(Clone, Copy, Debug, PartialEq)]
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
