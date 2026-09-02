use std::time::Duration;

use crate::Error;

/// Cap on extra sampled orders when there is no deadline. 100 is the knee
/// measured across benchmark graphs: fewer leaves quality on the table, more
/// costs construction time without improving the decomposition.
pub(super) const MAX_SAMPLING_RUNS: u64 = 100;

/// Larger budgeted runs keep exploring after the short-run sample cap.
const EXTENDED_SAMPLING_RUNS: u64 = 1_000;
pub(super) const DIVERSE_SAMPLING_RUNS: u64 = 3;
// A 4.75 s soft budget reaches the two-stage hard deadline at 9.5 s, leaving
// output headroom under a ten-second process limit.
const EXTENDED_SAMPLING_MIN_SOFT_BUDGET: Duration = Duration::from_millis(4_750);

/// Default soft deadline for the sampled-min-fill portfolio. The hard deadline
/// inside the elimination core is twice this.
const SAMPLED_MIN_FILL_TIMEOUT_MS: u64 = 1000;

pub(super) const MIN_FLOWCUTTER_CANDIDATE_MS: u64 = 50;

/// What a portfolio runs under, beyond the candidate list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct PortfolioConfig {
    pub(super) soft_budget: Option<Duration>,
    pub(super) hard_budget: Option<Duration>,
    pub(super) sampling_runs: u64,
    pub(super) diverse_sampling_runs: u64,
    pub(super) flowcutter_budget: Option<Duration>,
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

    /// Defaults for the standard candidate set: no deadline, up to 100 extra
    /// seeds, and no FlowCutter candidate.
    pub fn standard() -> Self {
        Self {
            soft_budget: None,
            hard_budget: None,
            sampling_runs: MAX_SAMPLING_RUNS,
            diverse_sampling_runs: 0,
            flowcutter_budget: None,
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
            diverse_sampling_runs: if extended { DIVERSE_SAMPLING_RUNS } else { 0 },
            flowcutter_budget: extended.then_some(budget),
        }
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
    Ok(())
}
