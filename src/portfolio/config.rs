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

/// Dimensions the hedge places the vertices in.
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
    pub fn eccentricity_dims(dims: &[usize]) -> Self {
        dims.iter().fold(Self::default(), |series, &dim| {
            series.then(HedgeWeights::eccentricity_at(dim))
        })
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
    /// The hedge the standard portfolio runs: one stage, on a ranking in three
    /// dimensions under the embedding's default round cap.
    pub const fn eccentricity() -> Self {
        Hedge::Passes(HedgeSeries::of(HedgeWeights::eccentricity()))
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
    }
}

impl Eq for PortfolioConfig {}

impl PortfolioConfig {
    /// Defaults for the sampled-min-fill candidate set: a 1 s soft deadline
    /// and up to 100 further seeds.
    pub fn sampled_min_fill() -> Self {
        Self {
            soft_budget: Some(Duration::from_millis(SAMPLED_MIN_FILL_TIMEOUT_MS)),
            hard_budget: None,
            sampling_runs: MAX_SAMPLING_RUNS,
            diverse_sampling_runs: 0,
            flowcutter_budget: None,
            hedge: Hedge::Off,
            hedge_reserve: DEFAULT_HEDGE_RESERVE,
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
    pub fn standard() -> Self {
        Self {
            soft_budget: None,
            hard_budget: None,
            sampling_runs: MAX_SAMPLING_RUNS,
            diverse_sampling_runs: 0,
            flowcutter_budget: None,
            hedge: DEFAULT_HEDGE,
            hedge_reserve: DEFAULT_HEDGE_RESERVE,
        }
    }

    /// Standard candidates under a soft wall-clock budget, with sampling
    /// effort and the trailing FlowCutter slot scaled for the corresponding
    /// hard window.
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
