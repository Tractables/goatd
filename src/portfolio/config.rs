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

/// What the standard portfolio hedges with unless the caller says otherwise.
const DEFAULT_HEDGE: Hedge = Hedge::eccentricity();

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
    /// The fixed orders that read the weights and the diverse pass run a
    /// second time on an eccentricity ranking, which the portfolio computes
    /// itself: the vertices placed in `dim` dimensions, most peripheral first.
    ///
    /// The plain candidates go first, on the caller's weights and the seeds
    /// they always had, and every ordinary restart stays plain on the seed
    /// sequence a portfolio without the hedge runs. Nothing repeats a
    /// deterministic order, which ignores weights.
    EccentricityPasses {
        /// Dimensions the placement has, at most
        /// [`MAX_DIM`](crate::embedding::MAX_DIM).
        dim: usize,
        /// Round cap on the placement. The rounds also stop at the portfolio's
        /// soft deadline.
        rounds: usize,
    },
}

impl Hedge {
    /// The hedge the standard portfolio runs: three dimensions, under the
    /// embedding's default round cap.
    pub const fn eccentricity() -> Self {
        Hedge::EccentricityPasses {
            dim: DEFAULT_HEDGE_DIM,
            rounds: DEFAULT_MAX_ROUNDS,
        }
    }
}

/// What a portfolio runs under, beyond the candidate list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct PortfolioConfig {
    pub(super) soft_budget: Option<Duration>,
    pub(super) hard_budget: Option<Duration>,
    pub(super) sampling_runs: u64,
    pub(super) diverse_sampling_runs: u64,
    pub(super) flowcutter_budget: Option<Duration>,
    pub(super) hedge: Hedge,
}

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
}

impl Default for PortfolioConfig {
    fn default() -> Self {
        Self::standard()
    }
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
    if let Hedge::EccentricityPasses { dim, rounds } = config.hedge {
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
    Ok(())
}
