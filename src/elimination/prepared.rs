use std::time::{Duration, Instant};

use super::{Order, engine, execution};
use crate::{Error, Graph, TreeDecomposition, deadline, meter};

/// Measurements for the graph reduction shared by subsequent candidate runs.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Preparation {
    /// Vertices in the input graph.
    pub original_vertices: usize,
    /// Active vertices after safe reductions.
    pub residual_vertices: usize,
    /// Real elapsed preparation time.
    pub elapsed: Duration,
    /// Work charged to the construction meter during preparation.
    pub work_units: u64,
}

/// Per-candidate elimination limits over an already prepared graph.
///
/// Budgets use the construction clock, starting at each [`Prepared::run`].
/// With no hard budget specified, the hard cutoff is twice the soft budget.
/// Limits are cooperative: graph setup and valid-tree reconstruction can finish
/// after a cutoff. A width bound prunes candidates; it is not a hardness claim.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct RunConfig {
    soft_budget: Option<Duration>,
    hard_budget: Option<Duration>,
    width_bound: Option<u32>,
    complete_on_deadline: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            soft_budget: None,
            hard_budget: None,
            width_bound: None,
            complete_on_deadline: true,
        }
    }
}

impl RunConfig {
    /// Set the soft budget and optional separate hard budget.
    pub const fn with_budgets(mut self, soft: Option<Duration>, hard: Option<Duration>) -> Self {
        self.soft_budget = soft;
        self.hard_budget = hard;
        self
    }

    /// Abort as soon as an elimination bag would give a width greater than
    /// `bound`; a candidate of width exactly `bound` still completes. `None`
    /// disables pruning.
    pub const fn with_width_bound(mut self, bound: Option<u32>) -> Self {
        self.width_bound = bound;
        self
    }

    /// Complete unfinished components after a cutoff when true (the default).
    /// When false, a cutoff returns no candidate and also bounds initial
    /// fill-count setup by the hard deadline.
    pub const fn with_deadline_completion(mut self, complete: bool) -> Self {
        self.complete_on_deadline = complete;
        self
    }
}

/// Which construction cutoff ended a candidate run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Cutoff {
    /// The soft cutoff stopped the chosen order.
    Soft,
    /// The hard cutoff stopped the run.
    Hard,
}

/// A completed original-graph decomposition or the reason no candidate exists.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunOutcome {
    /// The elimination order finished.
    Completed(TreeDecomposition),
    /// Unfinished components were completed after a cutoff.
    CompletedAtDeadline(Cutoff, TreeDecomposition),
    /// A cutoff stopped a run configured not to complete partial work.
    DeadlineAborted(Cutoff),
    /// An elimination bag exceeded the caller's width bound.
    WidthAborted,
}

/// Reusable graph reductions and scratch storage for caller-selected orders.
///
/// The handle borrows one immutable graph. Orders, seeds and sampling weights
/// may change between runs; every returned tree uses that graph's original
/// vertex space. A different graph or vertex numbering requires a new handle.
/// The standard portfolio already shares preparation internally.
pub struct Prepared<'g> {
    graph: &'g Graph,
    prebuilt: engine::Prebuilt,
    preparation: Preparation,
}

impl<'g> Prepared<'g> {
    /// Prepare one graph. `budget` bounds reductions using the construction
    /// clock; graph conversion and residual component discovery finish even
    /// after the cutoff. `None` lets reductions finish.
    ///
    /// # Errors
    /// Returns an error if the budget cannot be represented as a deadline.
    pub fn new(graph: &'g Graph, budget: Option<Duration>) -> Result<Self, Error> {
        let end = budget
            .map(|b| deadline::checked(meter::now(), b, "elimination preparation"))
            .transpose()?;
        Ok(Self::at_deadline(graph, end))
    }

    pub(super) fn at_deadline(graph: &'g Graph, end: Option<Instant>) -> Self {
        let start = Instant::now();
        let units = meter::units_spent();
        let prebuilt = engine::prebuild(graph, end);
        let preparation = Preparation {
            original_vertices: graph.num_vertices() as usize,
            residual_vertices: prebuilt.num_active(),
            elapsed: start.elapsed(),
            work_units: meter::units_spent().saturating_sub(units),
        };
        Self {
            graph,
            prebuilt,
            preparation,
        }
    }

    /// The preparation cost, charged once for this handle.
    pub fn preparation(&self) -> Preparation {
        self.preparation
    }

    /// Run another order, reusing reductions, cached fill counts and buffers.
    /// The caller retains and ranks completed candidates. Runs are independent:
    /// a previous abort or candidate does not install a width bound for the next.
    ///
    /// # Errors
    /// Returns an error for a sampled order with the wrong weight count, a hard
    /// budget without a soft budget, a hard budget smaller than the soft one,
    /// or an unrepresentable deadline. Validation happens before changing scratch.
    pub fn run(
        &mut self,
        order: Order<'_>,
        seed: u64,
        config: RunConfig,
    ) -> Result<RunOutcome, Error> {
        validate_order(self.graph, order)?;
        let deadlines = deadline::staged(
            meter::now(),
            config.soft_budget,
            config.hard_budget,
            "elimination",
        )?;
        self.run_at(order, seed, config, deadlines)
    }

    pub(super) fn run_at(
        &mut self,
        order: Order<'_>,
        seed: u64,
        config: RunConfig,
        deadlines: deadline::TwoStage,
    ) -> Result<RunOutcome, Error> {
        let result = engine::run_order_prebuilt(
            &mut self.prebuilt,
            engine::RunSpec {
                order,
                seed,
                sample_band: 0,
                update_order_ties: false,
                stop: execution::ElimStop {
                    soft_deadline: deadlines.soft,
                    hard_deadline: deadlines.hard,
                    width_bound: config.width_bound,
                },
                complete_on_deadline: config.complete_on_deadline,
                setup_deadline: if config.complete_on_deadline {
                    None
                } else {
                    deadlines.hard
                },
            },
        );
        let cutoff = |value| match value {
            execution::Cutoff::Soft => Cutoff::Soft,
            execution::Cutoff::Hard => Cutoff::Hard,
        };
        Ok(match result {
            engine::OrderRun::Completed(tree) => RunOutcome::Completed(tree),
            engine::OrderRun::CompletedAtDeadline(end, tree) => {
                RunOutcome::CompletedAtDeadline(cutoff(end), tree)
            }
            engine::OrderRun::DeadlineAborted(end) => RunOutcome::DeadlineAborted(cutoff(end)),
            engine::OrderRun::WidthAborted => RunOutcome::WidthAborted,
        })
    }
}

pub(super) fn validate_order(graph: &Graph, order: Order<'_>) -> Result<(), Error> {
    if let Some(weights) = order.tie_weights()
        && weights.len() != graph.num_vertices() as usize
    {
        return Err(Error::InvalidInput(format!(
            "sampled elimination has {} weights for {} vertices",
            weights.len(),
            graph.num_vertices()
        )));
    }
    Ok(())
}
