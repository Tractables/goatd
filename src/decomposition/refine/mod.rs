//! Refine a decomposition by splitting it around FlowCutter separators.
//!
//! Separator replacements project the current decomposition onto both sides and
//! glue the projections at a separator bag. The whole-pass wrapper accepts by
//! `(treewidth, total bag size)`; sessions leave acceptance to the caller.

use std::time::{Duration, Instant};

use super::TreeDecomposition;
use super::ops::{glue_at_separator, project_td_keeping_global_ids};
use crate::deadline::expired;
use crate::flowcutter::separator;
use crate::{Error, Graph};

#[cfg(test)]
mod tests;

mod session;
pub use session::{Config, Session};

/// FlowCutter restart breadth for one separator search — how many source/sink
/// pairs the anytime search tries per level.
const FLOWCUTTER_REFINEMENT_ITERATIONS: u32 = 50;

/// Per-level FlowCutter step budget = `all_vertices.len()` clamped to
/// `[MIN_REFINEMENT_STEPS, MAX_REFINEMENT_STEPS]`. A computation-step budget rather
/// than a wall-clock one is what makes the refined decomposition a pure
/// function of the graph, identical however loaded the machine is. A large
/// top-level subgraph gets the full budget;
/// deeper (smaller) subgraphs get proportionally fewer steps but, because
/// FlowCutter's internal per-iteration `step_cost` (~sqrt(n·m)) also shrinks,
/// still run enough iterations.
const REFINEMENT_STEPS_PER_VERTEX: u64 = 1;
const MIN_REFINEMENT_STEPS: u64 = 2_000;
const MAX_REFINEMENT_STEPS: u64 = 20_000;

/// Below this subgraph size, do not attempt another separator search.
const MIN_REFINEMENT_VERTICES: usize = 16;
/// Maximum number of nested separator replacements.
const MAX_RECURSION_DEPTH: u32 = 20;
/// Above this number of active vertices, a single FlowCutter iteration can
/// take seconds and is uninterruptible mid-iteration (the FlowCutter deadline check
/// only fires between iterations). Very large graphs can exceed this bound,
/// at which point post-process refinement reliably
/// overruns the deadline it was given. Above this gate, skip refinement and
/// return the input decomposition unchanged.
const MAX_VERTICES_FOR_REFINE: usize = 100_000;

/// Refine `td` by finding FlowCutter separators in `graph`, projecting `td`
/// onto each side of each separator, and recursing while the
/// refinement strictly improves `(width, total_bag_size)`.
///
/// `budget` is checked between separator searches and inside their search loops;
/// graph setup, cutter advances and decomposition reconstruction are indivisible.
/// It uses the construction clock, including charged work when the meter is armed;
/// it also arms a gate that skips subgraphs over 100 000 vertices, where one
/// uninterruptible search can run for seconds. The result is never worse than
/// `td` under `(width, total_bag_size)`.
///
/// # Errors
///
/// Returns an error if `td` is not a valid decomposition of `graph`, or if the
/// budget is too large to represent as a deadline.
pub fn refine_with_flowcutter(
    td: TreeDecomposition,
    graph: &Graph,
    budget: Option<Duration>,
) -> Result<TreeDecomposition, Error> {
    td.validate(graph)?;
    let deadline = budget
        .map(|budget| crate::deadline::checked(crate::meter::now(), budget, "refinement"))
        .transpose()?;
    let config = Config::default().with_vertex_limits(
        MIN_REFINEMENT_VERTICES,
        deadline.map(|_| MAX_VERTICES_FOR_REFINE),
    );
    let mut session = Session::trusted(graph, td, config)?;
    while let crate::decomposition::polishing::Advance::Proposal(proposal) =
        session.advance_legacy(deadline)
    {
        if proposal.recommended() {
            proposal.accept();
        }
    }
    Ok(session.into_tree())
}

fn to_global_vertices(local: &[u32], local_to_global: &[u32]) -> Vec<u32> {
    local
        .iter()
        .map(|&vertex| local_to_global[vertex as usize])
        .collect()
}
