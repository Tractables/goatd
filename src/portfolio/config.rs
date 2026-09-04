use std::time::Duration;

use crate::Error;
use crate::embedding::{DEFAULT_MAX_ROUNDS, MAX_DIM};

/// Cap on extra sampled orders when there is no deadline. 100 is the knee
/// measured across benchmark graphs: fewer leaves quality on the table, more
/// costs construction time without improving the decomposition.
pub(super) const MAX_SAMPLING_RUNS: u64 = 100;

/// Larger budgeted runs keep exploring after the short-run sample cap.
const EXTENDED_SAMPLING_RUNS: u64 = 1_000;
pub(super) const DIVERSE_INITIAL_COEFFICIENTS: [i8; 10] = [1, -1, -2, -3, -4, -5, -8, -7, -16, -32];
pub(super) const DIVERSE_REPLAY_COEFFICIENTS: [i8; 4] = [-3, -5, -8, -16];
const DIVERSE_REPLAY_SEEDS: u64 = 9;
/// The most diverse-score elimination orders the sampler has: one for each
/// initial degree coefficient, then each replay coefficient across its seeds.
/// [`PortfolioConfig::with_diverse_sampling_runs`] rejects anything above it.
pub const MAX_DIVERSE_SAMPLING_RUNS: u64 = DIVERSE_INITIAL_COEFFICIENTS.len() as u64
    + DIVERSE_REPLAY_COEFFICIENTS.len() as u64 * DIVERSE_REPLAY_SEEDS;
// A 4.75 s soft budget reaches the two-stage hard deadline at 9.5 s, leaving
// output headroom under a ten-second process limit.
const EXTENDED_SAMPLING_MIN_SOFT_BUDGET: Duration = Duration::from_millis(4_750);

/// Default soft deadline for the sampled-min-fill portfolio. The hard deadline
/// inside the elimination core is twice this.
const SAMPLED_MIN_FILL_TIMEOUT_MS: u64 = 1000;

pub(super) const MIN_FLOWCUTTER_CANDIDATE_MS: u64 = 50;

/// Residual size at or below which the standard portfolio runs the MCS-M
/// candidate. MCS-M costs one search per vertex over the whole residual, so its
/// cost grows with the vertex count times the edge count; above this it takes
/// more of the budget than the restarts it displaces are worth. On a corpus of
/// formula graphs it stays under a tenth of a second at this size and wins the
/// portfolio often; on larger residuals it costs a second or more and the
/// greedy orders were narrower anyway.
const DEFAULT_MINIMAL_TRIANGULATION_VERTICES: u32 = 1_000;

/// Graph size at or below which the standard portfolio minimalizes the
/// triangulation behind its winner. The pass holds two bitsets over the
/// vertices, so its memory grows with the square of this, and the time it
/// takes grows faster than the vertex count: on a corpus of formula graphs it
/// costs about half a second around this many vertices and several seconds at
/// three times as many, while the graphs it narrows are mostly the smaller
/// ones.
const DEFAULT_TRIANGULATION_REFINEMENT_VERTICES: u32 = 2_000;

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
/// A residual too large for the expensive orders runs sampled min-degree
/// restarts whatever is set here, so a hedge does not reach it.
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
    pub(super) minimal_triangulation: Option<u32>,
    pub(super) triangulation_refinement: Option<u32>,
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
            && self.minimal_triangulation == other.minimal_triangulation
            && self.triangulation_refinement == other.triangulation_refinement
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
            minimal_triangulation: None,
            triangulation_refinement: None,
        }
    }

    /// Request one trailing FlowCutter candidate with the given budget. It is
    /// skipped when the graph exceeds the backend's size limit or less than
    /// 50 ms remains in the portfolio's hard budget.
    pub fn with_flowcutter(mut self, budget: Duration) -> Self {
        self.flowcutter_budget = Some(budget);
        self
    }

    /// Set the soft portfolio budget, measured from before preprocessing. The
    /// hard deadline is twice this value.
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
    /// soft deadline with it on, the restarts run on past the count and the
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
    /// the portfolio runs. [`PortfolioConfig::standard_with_budget`] sets this
    /// along with a larger ordinary sample cap and a trailing FlowCutter
    /// candidate; set it here to take the diverse orders on their own.
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
    /// [`PortfolioConfig::with_restarts_to_deadline`] is turned on.
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
            minimal_triangulation: Some(DEFAULT_MINIMAL_TRIANGULATION_VERTICES),
            triangulation_refinement: Some(DEFAULT_TRIANGULATION_REFINEMENT_VERTICES),
        }
    }

    /// Standard candidates under a soft wall-clock budget, with sampling
    /// effort and the trailing FlowCutter slot scaled for the corresponding
    /// hard window.
    ///
    /// The hedge runs its first weighted stage on any budget and one more for
    /// as long as half of what the plain pass left holds another; what does not
    /// fit stays with the ordinary restarts. [`PortfolioConfig::with_hedge_reserve`]
    /// changes that fraction.
    ///
    /// The ordinary restarts run until the soft deadline: the sampling count
    /// caps how many seeds are drawn, not the clock, and a graph whose
    /// candidates are quick would otherwise finish the schedule with budget
    /// unspent. [`PortfolioConfig::with_restarts_to_deadline`] turned off
    /// stops them at the count instead.
    pub fn standard_with_budget(budget: Duration) -> Self {
        let extended = budget >= EXTENDED_SAMPLING_MIN_SOFT_BUDGET;
        let sampling_runs = if extended {
            EXTENDED_SAMPLING_RUNS
        } else {
            MAX_SAMPLING_RUNS
        };
        Self {
            soft_budget: Some(budget),
            hard_budget: None,
            sampling_runs,
            diverse_sampling_runs: if extended {
                MAX_DIVERSE_SAMPLING_RUNS
            } else {
                0
            },
            flowcutter_budget: extended.then_some(budget),
            hedge: DEFAULT_HEDGE,
            hedge_reserve: DEFAULT_HEDGE_RESERVE,
            restarts_to_deadline: true,
            minimal_triangulation: Some(DEFAULT_MINIMAL_TRIANGULATION_VERTICES),
            triangulation_refinement: Some(DEFAULT_TRIANGULATION_REFINEMENT_VERTICES),
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
    /// while the soft deadline has time left.
    ///
    /// On, the restarts carry on from the next seed of the same sequence and
    /// the soft deadline ends them; the count set by
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
    /// residual per vertex.
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
    /// graph's vertices, and because its time grows faster than that count: a
    /// wide gate spends what is left of the hard budget on the pass.
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
