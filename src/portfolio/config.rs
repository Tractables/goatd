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

/// What the sampled restarts leave of the hard window for the trailing
/// FlowCutter candidate when they run past the soft deadline.
pub(super) const FLOWCUTTER_RESERVE: Duration = Duration::from_millis(1_500);

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

/// How far above the minimum fill the ordinary restarts draw their tie set, in
/// fill edges. Drawing only from the vertices tied at the minimum leaves a
/// restart nothing to choose between on a graph where that set holds one
/// vertex at every step, so every seed replays one order; a band lets the
/// seeds separate. [`PortfolioConfig::with_sample_band`] with 0 restores the
/// exact minimum.
const DEFAULT_SAMPLE_BAND: u64 = 3;

/// Residuals of this size or smaller run the whole schedule on a run with no
/// soft budget. Under a budget the schedule is admitted by what a min-fill pass
/// over the residual is measured to cost, not by a vertex count; see
/// [`MIN_FILL_COST_MULTIPLE`] and [`FULL_SCHEDULE_PASSES`]. A run with no
/// budget has no window to measure a pass against, so the vertex line stands
/// there.
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
/// Fitted on 240 corpus graphs at a 4,750 ms soft budget to admit as many
/// residuals as the 10,000-vertex line it replaces, 141 against 139. Which ones
/// changes: 20 sparse residuals of 15,000 to 36,000 vertices, whose pass costs
/// 140 to 370 ms, now run the schedule, and 18 below 10,000 whose pass costs
/// 250 ms to 9 s no longer do.
pub(super) const FULL_SCHEDULE_PASSES: f64 = 15.0;

/// The default largest residual the expensive orders run on at all. Between the
/// full-schedule rule and this number they run on a paced schedule; above it the
/// portfolio keeps only its min-degree candidates.
/// [`PortfolioConfig::with_expensive_orders_up_to`] moves the upper line.
pub(super) const DEFAULT_MAX_RESIDUAL_FOR_EXPENSIVE_ORDERS: usize = 300_000;

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
    pub(super) expensive_orders_up_to: usize,
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
            && self.expensive_orders_up_to == other.expensive_orders_up_to
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
            expensive_orders_up_to: DEFAULT_MAX_RESIDUAL_FOR_EXPENSIVE_ORDERS,
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
            sample_band: DEFAULT_SAMPLE_BAND,
            sample_band_alternate: false,
            expensive_orders_up_to: DEFAULT_MAX_RESIDUAL_FOR_EXPENSIVE_ORDERS,
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
    /// The ordinary restarts run past the soft deadline into the hard window,
    /// stopping 1.5 s before the hard deadline so the trailing FlowCutter
    /// candidate still has that much to run in. Only a residual that runs the
    /// whole schedule does that; above 10,000 vertices the restarts stop at the
    /// soft deadline and the second stage stays with FlowCutter. The
    /// sampling count caps how many seeds are drawn, not the clock, and a
    /// graph whose candidates are quick would otherwise finish the schedule
    /// with budget unspent. One more restart starts only while what the
    /// previous one cost still fits before that stop.
    /// [`PortfolioConfig::with_restarts_to_deadline`] turned off stops them at
    /// the count instead.
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
            sample_band: DEFAULT_SAMPLE_BAND,
            sample_band_alternate: false,
            expensive_orders_up_to: DEFAULT_MAX_RESIDUAL_FOR_EXPENSIVE_ORDERS,
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
    /// trailing FlowCutter candidate on a residual that runs the whole
    /// schedule, and the soft deadline on any larger one.
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
    /// - min-fill runs to half the time the soft deadline has left when it
    ///   starts, rather than to the whole window, with the incumbent width
    ///   cutoff as everywhere else. An order that cannot finish gives the rest
    ///   back, and the restarts always start with time in hand;
    /// - nested dissection does not run: it reads its deadline between levels,
    ///   and one level's bisection of a graph with a million edges takes
    ///   seconds on its own;
    /// - the diverse pass and the hedge do not run;
    /// - the restarts are sampled min-fill when an initial min-fill produced a
    ///   decomposition and sampled min-degree when none did, and stop at the
    ///   soft deadline either way;
    /// - the trailing FlowCutter candidate runs as on any residual, under its
    ///   own vertex cap.
    ///
    /// Above the number given here the portfolio keeps only its min-degree
    /// candidates: the initial list drops min-fill and nested dissection after
    /// the first candidate, the diverse pass and the hedge do not run, and the
    /// ordinary restarts are sampled min-degree.
    ///
    /// Setting it to 10,000 or lower leaves no middle band, and every residual
    /// over the number runs min-degree only.
    pub fn with_expensive_orders_up_to(mut self, vertices: usize) -> Self {
        self.expensive_orders_up_to = vertices;
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
