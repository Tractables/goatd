use std::time::Duration;

use crate::Error;
use crate::embedding::{DEFAULT_MAX_ROUNDS, MAX_DIM};

/// Cap on extra sampled orders when there is no deadline. 100 is the knee
/// measured across benchmark graphs: fewer leaves quality on the table, more
/// costs construction time without improving the decomposition.
pub(super) const MAX_SAMPLING_RUNS: u64 = 100;

pub(super) const DIVERSE_INITIAL_COEFFICIENTS: [i8; 10] = [1, -1, -2, -3, -4, -5, -8, -7, -16, -32];
pub(super) const DIVERSE_REPLAY_COEFFICIENTS: [i8; 4] = [-3, -5, -8, -16];
const DIVERSE_REPLAY_SEEDS: u64 = 9;
/// The most diverse-score elimination orders the sampler has: one for each
/// initial degree coefficient, then each replay coefficient across its seeds.
/// [`PortfolioConfig::with_diverse_sampling_runs`] rejects anything above it.
pub const MAX_DIVERSE_SAMPLING_RUNS: u64 = DIVERSE_INITIAL_COEFFICIENTS.len() as u64
    + DIVERSE_REPLAY_COEFFICIENTS.len() as u64 * DIVERSE_REPLAY_SEEDS;

/// Default soft deadline for the sampled-min-fill portfolio. The hard deadline
/// inside the elimination core is twice this.
const SAMPLED_MIN_FILL_TIMEOUT_MS: u64 = 1000;

pub(super) const MIN_FLOWCUTTER_CANDIDATE_MS: u64 = 50;

/// What the sampled restarts leave of the hard window for the trailing
/// FlowCutter candidate when they run past the soft deadline.
pub(super) const FLOWCUTTER_RESERVE: Duration = Duration::from_millis(1_500);

/// Residual size at or below which the standard portfolio runs the MCS-M
/// candidate. MCS-M costs one search per vertex over the whole residual, so its
/// cost grows with the vertex count times the edge count; above this it takes
/// more of the budget than the restarts it displaces are worth. On a corpus of
/// formula graphs it stays under a tenth of a second at this size and wins the
/// portfolio often; on larger residuals it costs a second or more and the
/// greedy orders were narrower anyway.
const DEFAULT_MINIMAL_TRIANGULATION_VERTICES: u32 = 1_000;

/// Residual size at or below which the standard portfolio runs the maximum
/// cardinality search candidate. The plain search has no path walk to pay for,
/// so it costs one scan of the unnumbered vertices per vertex plus one pass
/// over the edges, and stays affordable on residuals far larger than MCS-M can
/// be run on: on a corpus of formula graphs it takes a couple of milliseconds
/// up to ten thousand vertices and about half a second above that, against a
/// budget of several seconds. Gates of two, ten and forty thousand were
/// compared on that corpus and the widest was the best on every reading,
/// because most of what the candidate wins is on residuals too large for the
/// greedy orders to run on at all.
const DEFAULT_MAXIMUM_CARDINALITY_VERTICES: u32 = 40_000;

/// Graph size at or below which the standard portfolio minimalizes the
/// triangulation behind its winner. The pass holds two bitsets over the
/// vertices, so its memory grows with the square of this, which is what the
/// gate is for. What the pass costs in time is not a function of the vertex
/// count at all — it follows the bags of the decomposition being rebuilt — so
/// the clock is what keeps it inside the budget, and this only keeps the memory
/// bounded.
const DEFAULT_TRIANGULATION_REFINEMENT_VERTICES: u32 = 2_000;

/// Graph size at or below which the standard budgeted portfolio recombines the
/// bags of its candidates. The search costs a pass over the graph per bag in
/// the pool, and the pool holds thousands, so above this the reserve it would
/// need is more of the window than the stage can be worth. It is the cheap
/// filter; the reserve priced against the pool is what settles the rest.
const DEFAULT_RECOMBINATION_VERTICES: u32 = 2_000;

/// The most of the hard window the recombination stage is given, taken off the
/// end so the rest of the schedule finishes that much earlier. It is given the
/// estimated cost of its own search where that is less.
pub(super) const RECOMBINATION_WINDOW_SHARE: u32 = 8;
/// The least the stage is given, whatever the estimate comes to.
pub(super) const MIN_RECOMBINATION_RESERVE: Duration = Duration::from_millis(50);
/// Passes over the pool the estimate pays for: the first search, and the growth
/// rounds after it where there is time for them.
pub(super) const RECOMBINATION_PASSES: u64 = 3;
/// What one pass of the search covers in a millisecond, in vertices and edges
/// of the graph per pool bag. Measured rather than derived: the pass allocates
/// a vertex list per component it cuts out and hashes each one, which costs far
/// more per edge than the work the meter is calibrated on.
pub(super) const RECOMBINATION_RATE_PER_MS: u64 = 12_000;
/// Where the pool slots of the extra draws start, past every stage's own.
pub(super) const VARIETY_SLOT: u32 = 400;

/// Dimensions the hedge places the vertices in, one weighted stage each, in
/// this order. Which graphs a dimension improves is close to arbitrary and two
/// dimensions improve mostly different ones, so a hedge that runs several
/// collects more of them. Three leads because on its own it is the dimension
/// that helps most: under a budget that fits one stage, that is the one to
/// spend it on. The series runs every dimension the embedding has, since the
/// budget rule stops it where there is no time for another stage and most
/// graphs finish the earlier stages with time to spare.
pub const DEFAULT_HEDGE_DIMS: [usize; 8] = [3, 1, 2, 4, 8, 5, 6, 7];

/// Dimensions the hedge places the vertices in when a caller asks for the
/// standard weighting without saying how many.
const DEFAULT_HEDGE_DIM: usize = 3;

/// Weighted stages one hedge can run. A series of eccentricity rankings has one
/// stage per dimension and the dimensions run 1..=[`MAX_DIM`] without repeats,
/// so eight is as long as such a series gets; the random control matches it.
pub const MAX_HEDGE_PASSES: usize = MAX_DIM;

/// What the standard portfolio hedges with unless the caller says otherwise.
const DEFAULT_HEDGE: Hedge = Hedge::eccentricity();

/// How much of the budget left after the plain pass the weighted stages after
/// the first may spend between them. Half: a stage costs about what the plain
/// pass cost, so on a budget that fits several the stages run, and on one where
/// the plain pass nearly filled the budget the restarts keep it.
///
/// A run with no soft budget has nothing to protect and runs every stage of the
/// series, whatever this says.
const DEFAULT_HEDGE_RESERVE: f64 = 0.5;

/// How much of the time the restart deadline has left the diverse pass is
/// admitted against: it runs while one more elimination of the kind the
/// initial orders just ran fits in this share of it. The same shape as the
/// hedge's reserve, and half for the same reason — a pass whose candidates
/// cost about what the initial orders cost gets a few of them in before the
/// restarts, and one whose candidates would fill the rest of the window does
/// not start.
pub(super) const DIVERSE_PASS_RESERVE: f64 = 0.5;

/// How far above the minimum fill the ordinary restarts draw their tie set, in
/// fill edges. Drawing only from the vertices tied at the minimum leaves a
/// restart nothing to choose between on a graph where that set holds one
/// vertex at every step, so every seed replays one order; a band lets the
/// seeds separate. [`PortfolioConfig::with_sample_band`] with 0 restores the
/// exact minimum.
const DEFAULT_SAMPLE_BAND: u64 = 3;

/// The share of its window the trailing FlowCutter candidate goes without
/// improving before the stall rule ends it, with
/// [`super::FLOWCUTTER_CANDIDATE_PATIENCE`] as the floor and the smaller of
/// what the rest of the schedule spent and
/// [`super::FLOWCUTTER_CANDIDATE_BASE_WINDOW`] as the ceiling. Half, so the tail is read the way the
/// restarts are: what it has already spent says how long it waits.
const TAIL_PATIENCE_SHARE: f64 = 0.5;

/// Whether the ordinary restarts give up on their own. See
/// [`SamplingPatience`].
///
/// Off in every configuration: the restarts are what the widths in this
/// library rest on, and a replay of the stall rule over a traced corpus run
/// loses width on about one small view in nine at every figure that saves
/// worthwhile time. A caller who would rather have the time asks for it with
/// [`PortfolioConfig::with_sampling_patience`].
const DEFAULT_SAMPLING_PATIENCE: SamplingPatience = SamplingPatience::Off;

/// When the portfolio stops drawing further sampled restarts.
///
/// The seeds converge. On a traced corpus run the restarts reach their best
/// width early and then repeat it for the rest of the list, so the tail of the
/// list costs the caller time and returns the tree it already had. This says
/// how long the portfolio waits for a restart to improve on what it holds
/// before it stops asking; the trailing FlowCutter candidate runs either way,
/// on the budget it was configured with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SamplingPatience {
    /// The restarts run to their count or their deadline, whichever comes
    /// first.
    Off,
    /// Stop once enough ordinary restarts have run and the last one that
    /// improved the portfolio's best decomposition — width first, then total
    /// bag size — is in the first half of them.
    ///
    /// The patience is a share of the restarts already run rather than a fixed
    /// count, so a longer budget, which fits more restarts, also waits longer
    /// before giving up. `min_restarts` is the floor under that share, and it
    /// is read against the restarts the schedule can actually draw: where the
    /// count is what stops them ([`PortfolioConfig::with_sampling_runs`],
    /// 100 by default), the floor is at most half that count, so the rule can
    /// still fire before the list runs out.
    ///
    /// Replaying the rule over a traced corpus run, 200 is where the width it
    /// costs stops falling faster than the time it saves: below it the loss
    /// grows about twice as fast as the saving. On the 100-restart schedule
    /// the same replay puts the floor at 50, which is what 200 becomes there.
    ///
    /// The trailing FlowCutter candidate is bounded the same way: above a
    /// short window it stops when it has gone half the window without
    /// improving, instead of running the window out.
    Halving {
        /// Restarts that always run before the rule can stop anything, at most
        /// half of what the schedule draws.
        min_restarts: u64,
    },
}

impl SamplingPatience {
    /// Whether the caller left the rule off, which is the default.
    pub(super) fn is_off(self) -> bool {
        matches!(self, SamplingPatience::Off)
    }

    /// Whether the restarts stop rather than run restart `index`, counting the
    /// ordinary restarts of this run from zero.
    ///
    /// `last_improvement` is the index of the last restart that improved the
    /// portfolio's best decomposition, and `None` where none has. A restart the
    /// incumbent width bound cut produced nothing, so it is not an improvement.
    /// `restarts` is how many the schedule draws, which the floor is read
    /// against.
    pub(super) fn stalled(self, index: u64, last_improvement: Option<u64>, restarts: u64) -> bool {
        match self {
            SamplingPatience::Off => false,
            SamplingPatience::Halving { min_restarts } => {
                index >= min_restarts.min(restarts / 2)
                    && last_improvement.is_none_or(|last| last < index / 2)
            }
        }
    }

    /// How long the trailing FlowCutter candidate goes without improving before
    /// it stops, on a window longer than [`FLOWCUTTER_CANDIDATE_BASE_WINDOW`].
    /// `None` leaves it running to the end of its window, which is what it does
    /// with the rule off.
    ///
    /// Half the window, and never longer than either what the rest of the
    /// schedule took before it or
    /// [`FLOWCUTTER_CANDIDATE_BASE_WINDOW`]. Without those two bounds a
    /// deadline an hour away leaves the tail an hour-long window and half an
    /// hour of patience, so the run spends the clock on a candidate that
    /// stopped improving in its first seconds. The schedule's own time is what
    /// keeps a quick graph quick; the base window is the absolute that does not
    /// grow with the deadline at all, and on a traced hour-long run of a 900
    /// vertex grid it is the one that matters — the restarts took 32.6 s there,
    /// so that bound alone would have left the tail another 34.8 s.
    ///
    /// [`FLOWCUTTER_CANDIDATE_BASE_WINDOW`]: super::FLOWCUTTER_CANDIDATE_BASE_WINDOW
    pub(super) fn tail_patience(self, window: Duration, spent: Duration) -> Option<Duration> {
        match self {
            SamplingPatience::Off => None,
            SamplingPatience::Halving { .. } => Some(
                window
                    .mul_f64(TAIL_PATIENCE_SHARE)
                    .min(spent)
                    .min(super::FLOWCUTTER_CANDIDATE_BASE_WINDOW)
                    .max(super::FLOWCUTTER_CANDIDATE_PATIENCE),
            ),
        }
    }
}

/// Residuals of this size or smaller run the whole schedule whatever the budget
/// is. Above the line the measurement decides: the schedule runs where a
/// min-fill pass over the residual is cheap enough for the budget to hold the
/// whole of it; see [`MIN_FILL_COST_MULTIPLE`] and [`FULL_SCHEDULE_PASSES`]. A
/// run with no soft budget has no window to measure a pass against, so the line
/// is the whole rule there.
pub(super) const MAX_RESIDUAL_FOR_FULL_SCHEDULE: usize = 10_000;

/// What a min-fill pass over the residual costs, as a multiple of what the
/// portfolio's first min-degree candidate cost. Both walk the same elimination
/// loop and differ in the score they keep, so the ratio between them is a
/// property of the graph rather than of the machine, and the machine's speed
/// cancels when one is estimated from the other.
///
/// Measured on 131 corpus graphs whose initial min-fill candidate finished
/// inside its window: the pass cost a median 6.7 times the first min-degree
/// candidate, with the middle eight tenths of them between 5.1 and 13.5.
pub(super) const MIN_FILL_COST_MULTIPLE: f64 = 6.7;

/// How many min-fill passes over the residual the whole schedule is worth. The
/// diverse pass alone is [`MAX_DIVERSE_SAMPLING_RUNS`] candidates and the hedge
/// and the restarts are more, so admitting the schedule wherever one pass fits
/// would admit it on residuals it cannot get through. The schedule runs while
/// the time the soft deadline has left holds this many estimated passes.
///
/// Fitted on 240 corpus graphs: at a 4,750 ms soft budget it adds 20 residuals
/// of 10,307 to 35,786 vertices, sparse enough that a pass over them is
/// estimated at 107 to 309 ms, to the 139 the vertex line already took. A
/// smaller number here admits residuals the schedule cannot get through; at the
/// same budget halving it takes those 20 additions to 32 and quartering it to
/// 39.
pub(super) const FULL_SCHEDULE_PASSES: f64 = 15.0;

/// The default largest residual the expensive orders run on by size alone.
/// Between the full-schedule rule and this number they run on a paced schedule
/// whatever a pass costs; above it they run where the budget pays for one, which
/// is [`PACED_SCHEDULE_PASSES`].
/// [`PortfolioConfig::with_expensive_orders_up_to`] moves the upper line.
pub(super) const DEFAULT_MAX_RESIDUAL_FOR_EXPENSIVE_ORDERS: usize = 300_000;

/// How many min-fill passes over the residual the paced schedule is worth: the
/// same test [`FULL_SCHEDULE_PASSES`] makes, at the fraction of the window the
/// paced schedule gets.
///
/// Above the line the paced schedule adds one min-fill order to the initial
/// loop, and the restarts follow it where it finished. That order runs to half
/// of what the restart deadline has left, so a pass the window cannot hold twice
/// is a pass that returns nothing, and the residual keeps the min-degree
/// candidates it has always had.
///
/// The line stays a floor: below it the paced schedule runs whatever a pass
/// costs, so this can only add residuals to the ones the line already took. It
/// adds none at the ten-second protocol, where a first min-degree candidate over
/// a residual of that size costs seconds of a 4.75-second budget and the
/// estimate is several times that again.
pub(super) const PACED_SCHEDULE_PASSES: f64 = 2.0;

/// Where one weighted stage takes its sampling weights from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HedgeWeights {
    /// Weights the portfolio derives itself: the vertices placed in `dim`
    /// dimensions and ranked by eccentricity, most peripheral first.
    Eccentricity {
        /// Dimensions the placement has, at most
        /// [`MAX_DIM`](crate::embedding::MAX_DIM).
        dim: usize,
        /// Round cap on the placement. The rounds also stop at the portfolio's
        /// soft deadline.
        rounds: usize,
    },
    /// Uniform weights drawn from a stream of the run's seed, carrying nothing
    /// about the graph. The control for the weightings that mean something: it
    /// perturbs the tie sets by as much as they do.
    Random {
        /// Which stream the weights come from. Streams differ from each other
        /// and from every other draw the run makes.
        stream: u64,
    },
}

impl HedgeWeights {
    /// Eccentricity weights in the dimensions and under the round cap the
    /// standard portfolio uses.
    pub const fn eccentricity() -> Self {
        HedgeWeights::Eccentricity {
            dim: DEFAULT_HEDGE_DIM,
            rounds: DEFAULT_MAX_ROUNDS,
        }
    }

    /// Eccentricity weights from a `dim`-dimensional placement, under the
    /// standard round cap.
    pub const fn eccentricity_at(dim: usize) -> Self {
        HedgeWeights::Eccentricity {
            dim,
            rounds: DEFAULT_MAX_ROUNDS,
        }
    }
}

impl Default for HedgeWeights {
    fn default() -> Self {
        HedgeWeights::eccentricity()
    }
}

/// The weightings a hedge runs its weighted stages on, one stage per entry, in
/// the order given.
///
/// Which graphs a weighting improves is close to arbitrary, and two weightings
/// improve mostly different ones, so running several collects more of them.
/// Each stage costs what one weighted stage costs, and the incumbent width
/// bounds the ones that follow.
#[derive(Clone, Copy, Debug, Default)]
#[must_use]
pub struct HedgeSeries {
    weights: [HedgeWeights; MAX_HEDGE_PASSES],
    len: u8,
    /// Set when more weightings were asked for than a series holds, so that the
    /// portfolio refuses the configuration instead of dropping them.
    overflow: bool,
}

/// Two series are equal when they run the same weightings in the same order.
/// The unused entries of a shorter series say nothing about it.
impl PartialEq for HedgeSeries {
    fn eq(&self, other: &Self) -> bool {
        self.overflow == other.overflow && self.weights() == other.weights()
    }
}

impl Eq for HedgeSeries {}

impl HedgeSeries {
    /// A series of one weighting.
    pub const fn of(first: HedgeWeights) -> Self {
        Self {
            weights: [first; MAX_HEDGE_PASSES],
            len: 1,
            overflow: false,
        }
    }

    /// Run `next` as a further weighted stage, after the ones already here.
    pub fn then(mut self, next: HedgeWeights) -> Self {
        match self.weights.get_mut(self.len as usize) {
            Some(slot) => {
                *slot = next;
                self.len += 1;
            }
            None => self.overflow = true,
        }
        self
    }

    /// One eccentricity weighting per dimension, in the order given, each under
    /// the standard round cap.
    pub const fn eccentricity_dims(dims: &[usize]) -> Self {
        let mut series = Self {
            weights: [HedgeWeights::eccentricity(); MAX_HEDGE_PASSES],
            len: 0,
            overflow: false,
        };
        let mut index = 0;
        while index < dims.len() {
            if index < MAX_HEDGE_PASSES {
                series.weights[index] = HedgeWeights::eccentricity_at(dims[index]);
                series.len += 1;
            } else {
                series.overflow = true;
            }
            index += 1;
        }
        series
    }

    /// `stages` weightings of random weights, on streams 0, 1, … — the control
    /// for a series of `stages` weightings that mean something.
    pub fn random(stages: usize) -> Self {
        (0..stages as u64).fold(Self::default(), |series, stream| {
            series.then(HedgeWeights::Random { stream })
        })
    }

    /// How many weighted stages the series runs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether the series runs no weighted stage at all, which no portfolio
    /// accepts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The weightings, in the order their stages run.
    pub(super) fn weights(&self) -> &[HedgeWeights] {
        &self.weights[..self.len as usize]
    }
}

/// Whether the portfolio runs the candidates that read sampling weights a
/// second time, on weights of its own.
///
/// Peripheral-first weights help some graphs and hurt others. Running them
/// against the candidates the portfolio would have run anyway, and keeping the
/// narrower result, costs the time of the extra candidates and nothing else.
///
/// A residual past the size the expensive orders run at runs restarts and
/// nothing else, whatever is set here, so a hedge does not reach it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Hedge {
    /// No hedge: every candidate runs once, on the caller's weights.
    Off,
    /// The fixed orders that read the weights and the diverse pass run again,
    /// once per weighting of the series and in the order the series gives them.
    ///
    /// The plain candidates go first, on the caller's weights and the seeds
    /// they always had, and every ordinary restart stays plain on the seed
    /// sequence a portfolio without the hedge runs. Nothing repeats a
    /// deterministic order, which ignores weights.
    Passes(HedgeSeries),
}

impl Hedge {
    /// The hedge the standard portfolio runs: one stage per dimension of
    /// [`DEFAULT_HEDGE_DIMS`], each ranking under the embedding's default round
    /// cap. Three comes first because it is the single dimension that helps
    /// most, and a run that only has room for one stage should spend it there.
    pub const fn eccentricity() -> Self {
        Hedge::Passes(HedgeSeries::eccentricity_dims(&DEFAULT_HEDGE_DIMS))
    }

    /// The weightings the weighted stages run, or `None` when nothing is
    /// hedged.
    pub(super) fn series(self) -> Option<HedgeSeries> {
        match self {
            Hedge::Off => None,
            Hedge::Passes(series) => Some(series),
        }
    }
}

/// What a portfolio runs under, beyond the candidate list.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct PortfolioConfig {
    pub(super) soft_budget: Option<Duration>,
    pub(super) hard_budget: Option<Duration>,
    pub(super) sampling_runs: u64,
    pub(super) diverse_sampling_runs: u64,
    pub(super) flowcutter_budget: Option<Duration>,
    pub(super) hedge: Hedge,
    pub(super) hedge_reserve: f64,
    pub(super) restarts_to_deadline: bool,
    pub(super) sample_band: u64,
    pub(super) sample_band_alternate: bool,
    pub(super) sampling_patience: SamplingPatience,
    pub(super) expensive_orders_up_to: usize,
    pub(super) maximum_cardinality: Option<u32>,
    pub(super) minimal_triangulation: Option<u32>,
    pub(super) triangulation_refinement: Option<u32>,
    pub(super) recombination: Option<u32>,
}

/// Two configurations are equal when they ask for the same run, the reserve
/// fraction compared by its bits so that a configuration can sit anywhere a
/// value compared by equality does.
impl PartialEq for PortfolioConfig {
    fn eq(&self, other: &Self) -> bool {
        self.soft_budget == other.soft_budget
            && self.hard_budget == other.hard_budget
            && self.sampling_runs == other.sampling_runs
            && self.diverse_sampling_runs == other.diverse_sampling_runs
            && self.flowcutter_budget == other.flowcutter_budget
            && self.hedge == other.hedge
            && self.hedge_reserve.to_bits() == other.hedge_reserve.to_bits()
            && self.restarts_to_deadline == other.restarts_to_deadline
            && self.sample_band == other.sample_band
            && self.sample_band_alternate == other.sample_band_alternate
            && self.sampling_patience == other.sampling_patience
            && self.expensive_orders_up_to == other.expensive_orders_up_to
            && self.maximum_cardinality == other.maximum_cardinality
            && self.minimal_triangulation == other.minimal_triangulation
            && self.triangulation_refinement == other.triangulation_refinement
            && self.recombination == other.recombination
    }
}

impl Eq for PortfolioConfig {}

impl PortfolioConfig {
    /// Defaults for the sampled-min-fill candidate set: a 1 s soft deadline
    /// and up to 100 further seeds. The deadline cuts the seeds short and the
    /// count stops them; every candidate is returned, so the count is also
    /// how many there can be.
    pub fn sampled_min_fill() -> Self {
        Self {
            soft_budget: Some(Duration::from_millis(SAMPLED_MIN_FILL_TIMEOUT_MS)),
            hard_budget: None,
            sampling_runs: MAX_SAMPLING_RUNS,
            diverse_sampling_runs: 0,
            flowcutter_budget: None,
            hedge: Hedge::Off,
            hedge_reserve: DEFAULT_HEDGE_RESERVE,
            restarts_to_deadline: false,
            sample_band: DEFAULT_SAMPLE_BAND,
            sample_band_alternate: false,
            sampling_patience: DEFAULT_SAMPLING_PATIENCE,
            expensive_orders_up_to: DEFAULT_MAX_RESIDUAL_FOR_EXPENSIVE_ORDERS,
            maximum_cardinality: None,
            minimal_triangulation: None,
            triangulation_refinement: None,
            recombination: None,
        }
    }

    /// Request one trailing FlowCutter candidate with the given budget. It is
    /// skipped when the graph exceeds the backend's size limit and when less
    /// than 50 ms remains in the portfolio's hard budget. It runs on residuals
    /// of every size otherwise.
    pub fn with_flowcutter(mut self, budget: Duration) -> Self {
        self.flowcutter_budget = Some(budget);
        self
    }

    /// Set the soft portfolio budget, measured from before preprocessing. The
    /// hard deadline is twice this value.
    ///
    /// On [`PortfolioConfig::standard`] this attaches a clock to the
    /// no-deadline schedule rather than switching to the budgeted one: no
    /// diverse pass, no trailing FlowCutter candidate, at most 100 restarts,
    /// and the run returns when those have run.
    /// [`PortfolioConfig::standard_with_budget`] is the schedule that spends a
    /// budget.
    pub fn with_soft_budget(mut self, budget: Duration) -> Self {
        self.soft_budget = Some(budget);
        self
    }

    /// Set the hard portfolio budget independently of the soft budget.
    ///
    /// Without this override, the hard budget is twice the soft budget. The
    /// hard budget must be at least the soft budget.
    pub fn with_hard_budget(mut self, budget: Duration) -> Self {
        self.hard_budget = Some(budget);
        self
    }

    /// Set the maximum number of ordinary sampled min-fill orders. Large
    /// residuals use sampled min-degree in their place.
    ///
    /// The count stops the restarts of a run with no soft deadline, and of
    /// one with [`PortfolioConfig::with_restarts_to_deadline`] off. Under a
    /// budget with it on, the restarts run on past the count and the restart
    /// deadline stops them.
    pub fn with_sampling_runs(mut self, runs: u64) -> Self {
        self.sampling_runs = runs;
        self
    }

    /// Set how many diverse-score elimination orders run before the ordinary
    /// sampled min-fill seeds. They vary the elimination score rather than the
    /// tie-breaking seed, so they reach orders repeated min-fill sampling does
    /// not. Large residuals run sampled min-degree instead and ignore this.
    ///
    /// Capped at [`MAX_DIVERSE_SAMPLING_RUNS`]; a larger value is rejected when
    /// the portfolio runs. This is how many candidates the pass has when it
    /// runs, not whether it runs: under a soft budget it also has to fit in
    /// the time the restart deadline has left.
    /// [`PortfolioConfig::standard_with_budget`] asks for the whole pass; set
    /// it here to take the diverse orders on their own.
    pub fn with_diverse_sampling_runs(mut self, runs: u64) -> Self {
        self.diverse_sampling_runs = runs;
        self
    }

    /// Defaults for the standard candidate set: no deadline, up to 100 extra
    /// seeds, no FlowCutter candidate, and the eccentricity hedge.
    ///
    /// With no deadline the hedge runs every stage of its series: the reserve
    /// exists to leave the restarts their time, and here there is no budget to
    /// take that time from. A stage is then only the fixed orders that read
    /// weights, since without a deadline the diverse pass does not run, so the
    /// whole series costs a handful of deterministic eliminations and the
    /// schedule does not depend on how long any of them took.
    ///
    /// A soft budget added with [`PortfolioConfig::with_soft_budget`] cuts the
    /// restarts short but does not extend them: the count stops them unless
    /// [`PortfolioConfig::with_restarts_to_deadline`] is turned on. This set
    /// under a budget is the no-deadline schedule wearing a clock — no diverse
    /// pass, no trailing FlowCutter candidate, at most 100 restarts — and it
    /// returns as soon as those have run, however much of the budget is left.
    /// A caller who wants the schedule the budget was measured for wants
    /// [`PortfolioConfig::standard_with_budget`].
    pub fn standard() -> Self {
        Self {
            soft_budget: None,
            hard_budget: None,
            sampling_runs: MAX_SAMPLING_RUNS,
            diverse_sampling_runs: 0,
            flowcutter_budget: None,
            hedge: DEFAULT_HEDGE,
            hedge_reserve: DEFAULT_HEDGE_RESERVE,
            restarts_to_deadline: false,
            sample_band: DEFAULT_SAMPLE_BAND,
            sample_band_alternate: false,
            sampling_patience: DEFAULT_SAMPLING_PATIENCE,
            expensive_orders_up_to: DEFAULT_MAX_RESIDUAL_FOR_EXPENSIVE_ORDERS,
            maximum_cardinality: Some(DEFAULT_MAXIMUM_CARDINALITY_VERTICES),
            minimal_triangulation: Some(DEFAULT_MINIMAL_TRIANGULATION_VERTICES),
            triangulation_refinement: Some(DEFAULT_TRIANGULATION_REFINEMENT_VERTICES),
            // The stage wants a share of a hard window, and this schedule has
            // no deadline to take one from.
            recombination: None,
        }
    }

    /// Standard candidates under a soft wall-clock budget. Every part of the
    /// schedule is offered on any budget and each decides against the clock
    /// whether it runs, so nothing here is switched by the size of the budget
    /// itself.
    ///
    /// The hedge runs its first weighted stage on any budget and one more for
    /// as long as half of what the plain pass left holds another; what does not
    /// fit stays with the ordinary restarts. [`PortfolioConfig::with_hedge_reserve`]
    /// changes that fraction. The diverse pass runs on a residual the schedule
    /// admits — 10,000 vertices or fewer, or above that line with a min-fill
    /// pass cheap enough for the budget to hold the passes the schedule is made
    /// of — and there while one more elimination of the kind the initial orders
    /// just ran fits in half the time the restart deadline has left. The
    /// trailing FlowCutter candidate runs while the window it is left is long
    /// enough to seed it and long enough for the backend's setup and first
    /// restart on this graph; on a window over 4.75 seconds that window is also
    /// what ends it, and on a shorter one it stops early once it has gone half a
    /// second without a narrower decomposition, or after 50 restarts.
    ///
    /// The ordinary restarts run past the soft deadline into the hard window,
    /// stopping 1.5 s before the hard deadline so the trailing FlowCutter
    /// candidate still has that much to run in. A residual past
    /// [`PortfolioConfig::with_expensive_orders_up_to`] does not: its restarts
    /// stop at the soft deadline and the second stage stays with FlowCutter —
    /// unless FlowCutter's own work model says it cannot start and stop inside
    /// that stage on a graph this size, in which case the elimination keeps the
    /// whole window and gives back only the time it needs to write its answer
    /// out. One more restart starts only while what the previous one cost still
    /// fits before that stop, so the deadline rather than the count is what
    /// ends them; the count is what a run stops at with
    /// [`PortfolioConfig::with_restarts_to_deadline`] turned off.
    ///
    /// A caller with a share of time per graph passes half of it here and the
    /// whole share to [`PortfolioConfig::with_hard_budget`]:
    ///
    /// ```
    /// # use std::time::Duration;
    /// # use goatd::portfolio::PortfolioConfig;
    /// # let share = Duration::from_secs(20);
    /// let config = PortfolioConfig::standard_with_budget(share / 2).with_hard_budget(share);
    /// ```
    ///
    /// The trailing FlowCutter candidate is the FlowCutter run of that share,
    /// and it is already inside it, so there is nothing left to refine
    /// afterwards. The run uses the whole share whether or not the restarts are
    /// still finding anything;
    /// [`PortfolioConfig::with_sampling_patience`] gives back what the
    /// restarts and the trailing candidate spend after they have stalled, at a
    /// cost in width.
    pub fn standard_with_budget(budget: Duration) -> Self {
        Self {
            soft_budget: Some(budget),
            hard_budget: None,
            sampling_runs: MAX_SAMPLING_RUNS,
            diverse_sampling_runs: MAX_DIVERSE_SAMPLING_RUNS,
            flowcutter_budget: Some(budget),
            hedge: DEFAULT_HEDGE,
            hedge_reserve: DEFAULT_HEDGE_RESERVE,
            restarts_to_deadline: true,
            sample_band: DEFAULT_SAMPLE_BAND,
            sample_band_alternate: false,
            sampling_patience: DEFAULT_SAMPLING_PATIENCE,
            expensive_orders_up_to: DEFAULT_MAX_RESIDUAL_FOR_EXPENSIVE_ORDERS,
            maximum_cardinality: Some(DEFAULT_MAXIMUM_CARDINALITY_VERTICES),
            minimal_triangulation: Some(DEFAULT_MINIMAL_TRIANGULATION_VERTICES),
            triangulation_refinement: Some(DEFAULT_TRIANGULATION_REFINEMENT_VERTICES),
            recombination: Some(DEFAULT_RECOMBINATION_VERTICES),
        }
    }

    /// Run the candidates that read sampling weights a second time on weights
    /// the portfolio ranks itself. [`Hedge::Off`] turns the standard
    /// portfolio's hedge off and leaves every candidate on the caller's
    /// weights.
    pub fn with_hedge(mut self, hedge: Hedge) -> Self {
        self.hedge = hedge;
        self
    }

    /// Set how much of the budget left after the plain pass the hedge's
    /// weighted stages may spend between them, as a fraction in `0 < f <= 1`.
    /// The rest is kept for the ordinary restarts.
    ///
    /// A stage is as many candidates as the plain diverse pass and it takes
    /// them from the restarts, so several stages can leave a large graph with
    /// no restarts at all. The portfolio has measured the plain pass by the
    /// time it decides, and runs one more stage only while that measurement
    /// fits in what the fraction leaves. The first stage runs whatever the
    /// fraction is, so this says nothing about a hedge of one weighting.
    /// Without a soft budget nothing binds and every stage runs.
    pub fn with_hedge_reserve(mut self, fraction: f64) -> Self {
        self.hedge_reserve = fraction;
        self
    }

    /// Whether the ordinary restarts keep drawing seeds past their count
    /// while the restart deadline has time left.
    ///
    /// The restart deadline is the hard deadline less the reserve kept for the
    /// trailing FlowCutter candidate. Over 10,000 vertices and under a soft
    /// budget over 4.75 seconds that reserve is the whole second stage, since
    /// the trailing candidate then runs to its window, so the restarts stop at
    /// the soft deadline there. On a residual past
    /// [`PortfolioConfig::with_expensive_orders_up_to`] it is the soft deadline
    /// instead, unless FlowCutter has declined the second stage, in which case
    /// it is the hard deadline less what handing the answer over costs.
    ///
    /// On, the restarts carry on from the next seed of the same sequence and
    /// the restart deadline ends them; the count set by
    /// [`PortfolioConfig::with_sampling_runs`] does not. Off, the restarts stop
    /// at the count or the deadline, whichever comes first. A run with no soft
    /// deadline stops at the count either way, since there is nothing else to
    /// stop at. [`PortfolioConfig::standard_with_budget`] turns this on;
    /// [`PortfolioConfig::standard`] and [`PortfolioConfig::sampled_min_fill`]
    /// leave it off.
    pub fn with_restarts_to_deadline(mut self, enabled: bool) -> Self {
        self.restarts_to_deadline = enabled;
        self
    }

    /// How far above the minimum fill the ordinary restarts draw their tie set,
    /// in fill edges.
    ///
    /// The restarts eliminate a vertex of minimum fill and break the tie at
    /// random. A band of `k` puts every vertex whose elimination adds at most
    /// `k` fill edges more than the best into the same draw, so seeds that
    /// would return the same order can separate. Every configuration starts
    /// from the same default band; 0 is the exact minimum. Only the restarts
    /// read it: the other candidates each run their own score's minimum.
    pub fn with_sample_band(mut self, band: u64) -> Self {
        self.sample_band = band;
        self
    }

    /// Alternate the ordinary restarts between the exact minimum and the band
    /// set by [`PortfolioConfig::with_sample_band`].
    ///
    /// On, an even-numbered restart draws from the vertices tied at the
    /// minimum and an odd-numbered one from the band. The seeds are the same
    /// sequence either way, so the even restarts are the candidates a
    /// portfolio with no band runs, seed for seed, and the odd ones are what
    /// the band adds. Off, every restart draws from the band.
    pub fn with_sample_band_alternate(mut self, alternate: bool) -> Self {
        self.sample_band_alternate = alternate;
        self
    }

    /// When the ordinary restarts give up on finding anything better, and with
    /// them the trailing FlowCutter candidate.
    ///
    /// The default, [`SamplingPatience::Off`], runs the whole list: the count
    /// set by [`PortfolioConfig::with_sampling_runs`], or the restart deadline
    /// where [`PortfolioConfig::with_restarts_to_deadline`] is on, and a
    /// trailing candidate that runs its window out.
    /// [`SamplingPatience::Halving`] stops both once they have stalled and
    /// gives the caller the rest of the budget back. It costs width: on a
    /// traced corpus run about one small view in nine came back wider, so it
    /// is for a caller who wants the time more than the last of the width.
    pub fn with_sampling_patience(mut self, patience: SamplingPatience) -> Self {
        self.sampling_patience = patience;
        self
    }

    /// The largest residual the expensive orders still run on, in vertices left
    /// after preprocessing. The default is 300,000. The residual size decides
    /// between three schedules.
    ///
    /// At or below 10,000 vertices the portfolio runs the whole schedule: every
    /// initial order, the diverse pass, the hedge, and sampled min-fill
    /// restarts. This lower boundary is fixed.
    ///
    /// Between 10,000 and the number given here the expensive orders still run,
    /// on terms that suit the size:
    ///
    /// - min-fill runs to half the time the restart deadline has left when it
    ///   starts, rather than to the whole window, with the incumbent width
    ///   cutoff as everywhere else. An order that cannot finish gives the rest
    ///   back, and the restarts always start with time in hand;
    /// - the loop over the initial candidates runs to the restart deadline
    ///   rather than to the soft one, so a first candidate that spends the
    ///   whole soft budget does not end the schedule. Each candidate's own
    ///   search still stops at the soft deadline, which is what leaves the
    ///   next one room;
    /// - nested dissection does not run: it reads its deadline between levels,
    ///   and one level's bisection of a graph with a million edges takes
    ///   seconds on its own;
    /// - the diverse pass and the hedge do not run;
    /// - the restarts are sampled min-fill when an initial min-fill produced a
    ///   decomposition and sampled min-degree when none did, and run to the
    ///   restart deadline either way;
    /// - the trailing FlowCutter candidate runs as on any residual, under its
    ///   own vertex cap.
    ///
    /// The restart deadline those first two read is the hard deadline less what
    /// the trailing FlowCutter candidate can use. Up to a 4.75-second soft
    /// budget that candidate stops long before its window and the reserve is
    /// 1.5 seconds, so the schedule gets the rest of the second stage. Over it
    /// the window is what ends the candidate and the reserve is the whole
    /// second stage, so the schedule stops at the soft deadline and the stage is
    /// the candidate's.
    ///
    /// Above the number given here the size does not settle the schedule on its
    /// own: the portfolio runs its first min-degree candidate, prices a min-fill pass
    /// over the residual from what that cost, and runs the paced schedule where
    /// the time the soft deadline has left holds two of those passes, which is
    /// what the min-fill order it adds would run to. Where it does not, only the
    /// min-degree candidates are left: the initial list drops min-fill and
    /// nested dissection after the first candidate, the diverse pass and the
    /// hedge do not run, and the ordinary restarts are sampled min-degree. A run
    /// with no soft budget has no window to price a pass against, so the number
    /// given here is the whole rule there.
    ///
    /// The candidates carrying a vertex cap of their own are not part of this
    /// choice, the way the trailing FlowCutter candidate already was not: the
    /// two cardinality searches and the fill-dropping pass each answer their own
    /// gate, and every one of those gates sits far below the default limit here.
    ///
    /// Setting it to 10,000 or lower leaves no middle band, and every residual
    /// over the number runs min-degree plus whatever those gates admit, unless
    /// the budget pays for a paced pass over it.
    pub fn with_expensive_orders_up_to(mut self, vertices: usize) -> Self {
        self.expensive_orders_up_to = vertices;
        self
    }

    /// Run the maximum cardinality search candidate while the preprocessed
    /// residual has at most `max_residual_vertices` vertices.
    ///
    /// The search numbers the residual from `n` down to 1, always taking a
    /// vertex with the most numbered neighbours, and the candidate eliminates
    /// along that numbering reversed. It reads no seed and no weights, so it is
    /// one candidate; it runs before the MCS-M candidate and before the
    /// restarts, and it stops at the soft deadline rather than taking their
    /// time. It adds no fill on a chordal residual and no minimality guarantee
    /// on any other, so it is a cheap construction to race against the greedy
    /// orders rather than a better one.
    ///
    /// The gate is a vertex count because the search scans the unnumbered
    /// vertices once per vertex. The soft deadline is what stops it: the search
    /// reads the clock while it walks, so a residual the gate lets through but
    /// the budget cannot finish gives up part-way and the portfolio keeps what
    /// the other candidates found.
    pub fn with_maximum_cardinality(mut self, max_residual_vertices: u32) -> Self {
        self.maximum_cardinality = Some(max_residual_vertices);
        self
    }

    /// Run no maximum cardinality search candidate.
    pub fn without_maximum_cardinality(mut self) -> Self {
        self.maximum_cardinality = None;
        self
    }

    /// Run the MCS-M candidate while the preprocessed residual has at most
    /// `max_residual_vertices` vertices.
    ///
    /// MCS-M eliminates along a numbering that fills the residual to a minimal
    /// triangulation. It is one deterministic candidate, it runs after the
    /// fixed orders and before the restarts, and it stops at the soft deadline
    /// with nothing rather than taking their time. On most graphs it is wider
    /// than the greedy orders and the portfolio keeps whichever is narrower;
    /// where it wins it wins by several.
    ///
    /// The gate is a vertex count because the search costs one traversal of the
    /// residual per vertex. The soft deadline is what stops it: the search reads
    /// the clock while it walks, so a residual the gate lets through but the
    /// budget cannot finish gives up part-way and the portfolio keeps what the
    /// other candidates found.
    pub fn with_minimal_triangulation(mut self, max_residual_vertices: u32) -> Self {
        self.minimal_triangulation = Some(max_residual_vertices);
        self
    }

    /// Run no MCS-M candidate.
    pub fn without_minimal_triangulation(mut self) -> Self {
        self.minimal_triangulation = None;
        self
    }

    /// Minimalize the triangulation behind the portfolio's winner on graphs of
    /// at most `max_vertices` vertices.
    ///
    /// The winner's bags are completed to cliques, the added edges that can go
    /// without breaking chordality are dropped, and the cliques of what remains
    /// become the new bags. The pass never widens the decomposition; where it
    /// drops nothing, or improves neither the width nor the total bag size, the
    /// winner is returned unchanged.
    ///
    /// The gate is a vertex count because the pass holds two bitsets over the
    /// graph's vertices. It is not what keeps the pass inside the budget: the
    /// pass costs about what completing the winner's bags costs, which the
    /// winner says in advance, so the portfolio runs it only while that fits in
    /// what is left of the hard deadline and stops it there if the sweeps run
    /// long.
    pub fn with_triangulation_refinement(mut self, max_vertices: u32) -> Self {
        self.triangulation_refinement = Some(max_vertices);
        self
    }

    /// Leave the winner's triangulation as the candidate that produced it left
    /// it.
    pub fn without_triangulation_refinement(mut self) -> Self {
        self.triangulation_refinement = None;
        self
    }

    /// Recombine the bags of the candidates on graphs of at most
    /// `max_vertices` vertices.
    ///
    /// The last stage of the schedule collects the bags of every decomposition
    /// the run produced and searches over that pool for the narrowest tree
    /// decomposition whose bags all come from it. The pool holds the winner's
    /// own bags, so the search cannot come back wider, and the portfolio keeps
    /// the result only where it is narrower.
    ///
    /// The stage is given what its search is estimated to cost, never more than
    /// a share of the hard window, and it is taken off the end, so every other
    /// candidate stops that much earlier. A run with no budget at all has no
    /// window to take a share of, and does not run the stage. The gate is a
    /// vertex count because the search costs a pass over the graph per bag in
    /// the pool: above it the reserve the stage would need is more of the
    /// window than it can be worth. What the search holds is capped separately,
    /// by a constant the graph's size does not enter.
    pub fn with_recombination(mut self, max_vertices: u32) -> Self {
        self.recombination = Some(max_vertices);
        self
    }

    /// Return the best single candidate instead of recombining the bags of all
    /// of them.
    pub fn without_recombination(mut self) -> Self {
        self.recombination = None;
        self
    }
}

impl Default for PortfolioConfig {
    fn default() -> Self {
        Self::standard()
    }
}

/// A hedge's weightings: at least one, no more than the series holds, none
/// repeated, and each one usable.
fn validate_hedge_series(series: HedgeSeries) -> Result<(), Error> {
    if series.overflow {
        return Err(Error::InvalidInput(format!(
            "portfolio hedge runs at most {MAX_HEDGE_PASSES} modified passes"
        )));
    }
    let weights = series.weights();
    if weights.is_empty() {
        return Err(Error::InvalidInput(
            "portfolio hedge needs at least one modified pass".into(),
        ));
    }
    for (index, &entry) in weights.iter().enumerate() {
        if weights[..index].contains(&entry) {
            return Err(Error::InvalidInput(format!(
                "portfolio hedge runs {entry:?} twice, which is the same candidates twice"
            )));
        }
        match entry {
            HedgeWeights::Eccentricity { dim, rounds } => {
                if dim == 0 || dim > MAX_DIM {
                    return Err(Error::InvalidInput(format!(
                        "portfolio hedge dimension {dim} is outside 1..={MAX_DIM}"
                    )));
                }
                if rounds == 0 {
                    return Err(Error::InvalidInput(
                        "portfolio hedge placement needs at least one round".into(),
                    ));
                }
            }
            HedgeWeights::Random { .. } => {}
        }
    }
    Ok(())
}

pub(super) fn validate(config: PortfolioConfig) -> Result<(), Error> {
    if let Some(budget) = config.flowcutter_budget
        && budget < Duration::from_millis(MIN_FLOWCUTTER_CANDIDATE_MS)
    {
        return Err(Error::InvalidInput(format!(
            "portfolio FlowCutter budget must be at least {MIN_FLOWCUTTER_CANDIDATE_MS} ms"
        )));
    }
    if config
        .flowcutter_budget
        .is_some_and(|budget| budget.as_millis() > i64::MAX as u128)
    {
        return Err(Error::InvalidInput(
            "portfolio FlowCutter budget does not fit in milliseconds".into(),
        ));
    }
    if config.diverse_sampling_runs > MAX_DIVERSE_SAMPLING_RUNS {
        return Err(Error::InvalidInput(format!(
            "portfolio diverse sampling runs must be at most {MAX_DIVERSE_SAMPLING_RUNS}"
        )));
    }
    if let Some(series) = config.hedge.series() {
        validate_hedge_series(series)?;
    }
    if !(config.hedge_reserve.is_finite()
        && config.hedge_reserve > 0.0
        && config.hedge_reserve <= 1.0)
    {
        return Err(Error::InvalidInput(format!(
            "portfolio hedge reserve {} is not a fraction in 0 < f <= 1",
            config.hedge_reserve
        )));
    }
    Ok(())
}
