//! FlowCutter tree-decomposition construction and separator search.
//!
//! [`decompose`] uses the vendored PACE 2017 C++ tree-decomposition solver.
//! [`separator`] provides the related Rust separator search used by
//! decomposition refinement. Step budgets are repeatable; elapsed-time budgets
//! may stop at different iterations as machine speed and load change.

use std::os::raw::c_int;
use std::time::Duration;

use crate::{Error, Graph, TreeDecomposition};

mod native;
pub mod separator;

/// Whether an elapsed-time limit also adapts the search to a short window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeoutBehavior {
    /// Stop at the limit without changing the step search's heuristic limits.
    StopOnly,
    /// Shorten expensive prepasses as well as stopping at the limit.
    AdaptSearch,
}

impl TimeoutBehavior {
    fn as_ffi(self) -> c_int {
        match self {
            Self::StopOnly => 0,
            Self::AdaptSearch => 1,
        }
    }
}

/// Limits for one vendored FlowCutter run.
///
/// Use [`Budget::steps`] for a repeatable computation limit or
/// [`Budget::timed`] for an elapsed-time limit. [`Budget::with_timeout`] adds
/// an elapsed-time safety limit to an existing step budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct Budget {
    kind: BudgetKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BudgetKind {
    Timed {
        timeout: Duration,
        patience: Option<Duration>,
        iterations: u32,
        steps: u64,
        timeout_behavior: TimeoutBehavior,
    },
    Steps {
        steps: u64,
        iterations: u32,
    },
}

/// Step ceiling carried by a purely timed run. The elapsed-time limit normally
/// ends the search first.
pub(crate) const FC_TIMED_STEPS: u64 = 1_000_000;

impl Budget {
    /// Create an elapsed-time budget. `patience` stops a run that has not
    /// improved for that long. The search adapts expensive prepasses to the
    /// available time. Nonzero sub-millisecond durations are rounded up to one
    /// millisecond at the native boundary.
    pub const fn timed(timeout: Duration, patience: Option<Duration>, iterations: u32) -> Self {
        Self {
            kind: BudgetKind::Timed {
                timeout,
                patience,
                iterations,
                steps: FC_TIMED_STEPS,
                timeout_behavior: TimeoutBehavior::AdaptSearch,
            },
        }
    }

    /// Create a repeatable step and iteration budget.
    pub const fn steps(steps: u64, iterations: u32) -> Self {
        Self {
            kind: BudgetKind::Steps { steps, iterations },
        }
    }

    /// Add or replace an elapsed-time limit without changing the step or
    /// iteration ceiling.
    pub const fn with_timeout(
        self,
        timeout: Duration,
        patience: Option<Duration>,
        behavior: TimeoutBehavior,
    ) -> Self {
        let (steps, iterations) = match self.kind {
            BudgetKind::Timed {
                steps, iterations, ..
            }
            | BudgetKind::Steps { steps, iterations } => (steps, iterations),
        };
        Self {
            kind: BudgetKind::Timed {
                timeout,
                patience,
                iterations,
                steps,
                timeout_behavior: behavior,
            },
        }
    }
}

pub(super) fn duration_ms(duration: Duration) -> i64 {
    duration.as_millis().max(1) as i64
}

/// Largest graph the vendored backend is handed. Its graph representation
/// allocates a quadratic adjacency bitset.
const MAX_VERTICES: u32 = 100_000;

/// Largest edge list the vendored backend is handed before allocation failure
/// becomes likely to terminate the process across the C boundary.
const MAX_EDGES: usize = 20_000_000;

fn vendor_size_guard(graph: &Graph) -> Result<(), Error> {
    vendor_size_guard_counts(graph.num_vertices, graph.edges.len())
}

fn vendor_size_guard_counts(num_vertices: u32, num_edges: usize) -> Result<(), Error> {
    if num_vertices > MAX_VERTICES {
        return Err(Error::TooLarge(format!(
            "graph too large for FlowCutter ({num_vertices} vertices; its quadratic adjacency matrix would exceed memory)"
        )));
    }
    if num_edges > MAX_EDGES {
        return Err(Error::TooLarge(format!(
            "graph too dense for FlowCutter ({num_edges} edges; the backend would exceed memory)"
        )));
    }
    Ok(())
}

/// Run the vendored C++ FlowCutter and return its decomposition.
///
/// # Errors
///
/// Returns an error for zero or unrepresentable work limits, an input over the
/// backend's allocation guards, no backend result, or an invalid backend
/// result.
pub fn decompose(graph: &Graph, budget: Budget) -> Result<TreeDecomposition, Error> {
    vendor_size_guard(graph)?;
    validate_budget(budget)?;
    if graph.num_vertices == 0 {
        return Ok(TreeDecomposition::from_parts(0, Vec::new(), Vec::new()));
    }
    native::run(graph, budget)
}

fn validate_budget(budget: Budget) -> Result<(), Error> {
    match budget.kind {
        BudgetKind::Timed {
            timeout,
            patience,
            iterations,
            steps,
            ..
        } if timeout.is_zero()
            || patience.is_some_and(|patience| patience.is_zero())
            || iterations == 0
            || steps == 0 =>
        {
            return Err(Error::InvalidInput(
                "a timed FlowCutter run needs positive time, steps, iterations, and patience when present"
                    .into(),
            ));
        }
        BudgetKind::Steps { steps, iterations } if steps == 0 || iterations == 0 => {
            return Err(Error::InvalidInput(
                "a step-budgeted FlowCutter run needs positive steps and iterations".into(),
            ));
        }
        _ => {}
    }

    let (steps, iterations) = match budget.kind {
        BudgetKind::Timed {
            timeout,
            patience,
            steps,
            iterations,
            ..
        } => {
            if timeout.as_millis() > i64::MAX as u128
                || patience.is_some_and(|value| value.as_millis() > i64::MAX as u128)
            {
                return Err(Error::InvalidInput(
                    "FlowCutter duration does not fit in milliseconds".into(),
                ));
            }
            (steps, iterations)
        }
        BudgetKind::Steps { steps, iterations } => (steps, iterations),
    };
    if steps > i64::MAX as u64 {
        return Err(Error::InvalidInput(
            "FlowCutter step budget does not fit in i64".into(),
        ));
    }
    if iterations > i32::MAX as u32 {
        return Err(Error::InvalidInput(
            "FlowCutter iteration count does not fit in i32".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
