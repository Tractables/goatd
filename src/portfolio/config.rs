use std::time::Duration;

use crate::Error;

/// Cap on extra sampled orders when there is no deadline. 100 is the knee
/// measured across benchmark graphs: fewer leaves quality on the table, more
/// costs construction time without improving the decomposition.
pub(super) const MAX_SAMPLING_RUNS: u64 = 100;

/// Default soft deadline for the sampled-min-fill portfolio. The hard deadline
/// inside the elimination core is twice this.
const SAMPLED_MIN_FILL_TIMEOUT_MS: u64 = 1000;

pub(super) const MIN_FLOWCUTTER_CANDIDATE_MS: u64 = 50;

/// What a portfolio runs under, beyond the candidate list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct PortfolioConfig {
    pub(super) soft_budget: Option<Duration>,
    pub(super) sampling_runs: u64,
    pub(super) flowcutter_budget: Option<Duration>,
}

impl PortfolioConfig {
    /// Defaults for the sampled-min-fill candidate set: a 1 s soft deadline
    /// and up to 100 further seeds.
    pub fn sampled_min_fill() -> Self {
        Self {
            soft_budget: Some(Duration::from_millis(SAMPLED_MIN_FILL_TIMEOUT_MS)),
            sampling_runs: MAX_SAMPLING_RUNS,
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

    /// Set the maximum number of extra sampled elimination orders.
    pub fn with_sampling_runs(mut self, runs: u64) -> Self {
        self.sampling_runs = runs;
        self
    }

    /// Defaults for the standard candidate set: no deadline, up to 100 extra
    /// seeds, and no FlowCutter candidate.
    pub fn standard() -> Self {
        Self {
            soft_budget: None,
            sampling_runs: MAX_SAMPLING_RUNS,
            flowcutter_budget: None,
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
