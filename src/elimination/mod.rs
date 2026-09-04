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

use execution::ElimStop;

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
    if let Some(weights) = order.tie_weights()
        && weights.len() != graph.num_vertices as usize
    {
        return Err(crate::Error::InvalidInput(format!(
            "sampled elimination has {} weights for {} vertices",
            weights.len(),
            graph.num_vertices
        )));
    }
    let deadlines = crate::deadline::two_stage(crate::meter::now(), soft_budget, "elimination")?;
    let mut prebuilt = engine::prebuild(graph, deadlines.soft);
    let run = engine::run_order_prebuilt(
        &mut prebuilt,
        engine::RunSpec {
            order,
            seed,
            // A single order samples the exact minimum; the band is a
            // portfolio setting.
            sample_band: 0,
            update_order_ties: false,
            stop: ElimStop {
                soft_deadline: deadlines.soft,
                hard_deadline: deadlines.hard,
                width_bound: None,
            },
            // Always produce a valid TD.
            complete_on_deadline: true,
            setup_deadline: None,
        },
    );
    match run {
        engine::OrderRun::Completed(decomposition)
        | engine::OrderRun::CompletedAtDeadline(_, decomposition) => Ok(decomposition),
        engine::OrderRun::DeadlineAborted(_) | engine::OrderRun::WidthAborted => {
            unreachable!("a deadline-completing, unbounded run must produce a decomposition")
        }
    }
}
