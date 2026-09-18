//! Elimination-order tree decompositions: safe-reduction preprocessing, then
//! an elimination order, then the bag tree that order induces.
//!
//! Every construction starts from the same preprocessing and then picks an
//! order — greedily, or from a separator recursion, or from a cardinality
//! search. [`Order`] holds them all:
//!
//!   * **min-fill** and **min-degree** — the plain greedy orders, ties broken
//!     by a seeded salt.
//!   * **min-fill and min-degree with weighted tie-set sampling** — fill-only
//!     (resp. degree-only) priority, the whole tie set sampled by a per-vertex
//!     weight the caller supplies. These two are the orders the portfolios
//!     run.
//!   * **nested dissection** via
//!     [`multilevel_graph_bisect`](crate::partition::multilevel_graph_bisect),
//!     separating on a König-Egerváry minimum vertex cover.
//!   * **minimal triangulation** — the MCS-M numbering, which fills the
//!     residual to a triangulation no added edge can be dropped from.
//!   * **maximum cardinality** — the plain MCS numbering, the same search with
//!     a shorter reach and no minimality guarantee.
//!
//! [`decompose`] runs one order once. [`crate::portfolio`] combines
//! several orders, [`crate::decomposition::refine_with_flowcutter`] improves an
//! existing decomposition with FlowCutter separators, and
//! [`crate::decomposition::minimalize_triangulation`] drops the fill edges its
//! bags do not need.

use std::time::Duration;

pub(crate) mod build_td;
pub(crate) mod engine;
pub(crate) mod execution;
mod graph;
mod greedy;
pub(crate) mod minimal_triangulation;
mod nested_dissection;
mod order;
mod preprocess;
mod vertex_cover_separator;

#[cfg(test)]
mod tests;

mod prepared;
pub use prepared::{Cutoff, Preparation, Prepared, RunConfig, RunOutcome};

pub use order::Order;

/// Run one elimination order over `graph` and return its tree decomposition.
///
/// `seed` drives the salt the deterministic orders break ties with and the
/// draws the sampling orders make; one seed gives one decomposition. A
/// sampling order's weight must have one entry per vertex of `graph`.
///
/// `soft_budget`, measured from before preprocessing, sets two cutoffs.
/// Deterministic min-fill and min-degree switch to cheaper stale-heap scoring
/// at the first cutoff. Sampled orders and nested dissection have no equivalent
/// cheap mode and continue unchanged. At twice the budget, every order stops
/// and puts each unfinished residual component in one bag, so a valid result
/// is always produced without quadratic deadline output. `None` runs to
/// completion.
///
/// # Errors
///
/// Returns an error when a sampled order's weight count differs from the graph
/// vertex count, or when the budget cannot be represented as a deadline.
pub fn decompose(
    graph: &crate::Graph,
    order: Order<'_>,
    seed: u64,
    soft_budget: Option<Duration>,
) -> Result<crate::TreeDecomposition, crate::Error> {
    prepared::validate_order(graph, order)?;
    let deadlines = crate::deadline::staged(crate::meter::now(), soft_budget, None, "elimination")?;
    let mut prepared = Prepared::at_deadline(graph, deadlines.soft);
    match prepared.run_at(order, seed, RunConfig::default(), deadlines)? {
        RunOutcome::Completed(tree) | RunOutcome::CompletedAtDeadline(_, tree) => Ok(tree),
        RunOutcome::DeadlineAborted(_) | RunOutcome::WidthAborted => {
            unreachable!("a deadline-completing, unbounded run must produce a decomposition")
        }
    }
}
