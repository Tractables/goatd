//! Elimination-order tree decompositions: safe-reduction preprocessing, then
//! a greedy or nested-dissection elimination order, then the bag tree that
//! order induces.
//!
//! Every construction starts from the same preprocessing and then picks an
//! order — or, for nested dissection, derives one from a separator
//! recursion. [`Config`] holds the five that exist:
//!
//!   * **min-fill** and **min-degree** — the plain greedy orders, ties broken
//!     by a seeded salt.
//!   * **min-fill and min-degree with weighted tie-set sampling** — fill-only
//!     (resp. degree-only) priority, the whole tie set sampled by a per-vertex
//!     weight the caller supplies. These two are the orders the portfolios
//!     run.
//!   * **nested dissection** via [`multilevel_bisect`](crate::multilevel_bisect),
//!     separating on a König-Egerváry minimum vertex cover.
//!
//! [`elimination_td`] runs one order once. The portfolio functions run several
//! `(order, seed)` pairs under a wall-clock budget and, on the refined path,
//! finish with FlowCutter-cut refinement ([`refine_td_with_flowcutter_cut`]).

use std::time::Duration;

mod build_td;
mod flow_cut;
mod graph;
mod minfill_core;
mod nested_diss;
mod portfolio;
mod preprocess;
mod refine;
mod width_opt;

#[cfg(test)]
mod tests;

use minfill_core::ElimStop;

pub use portfolio::{
    PortfolioConfig, five_slot_portfolio, refined_select_key, refined_td, single_slot_portfolio,
};
pub use refine::refine_td_with_flowcutter_cut;
pub use width_opt::Config;

/// Run one elimination order over `graph` and return its tree decomposition.
///
/// `seed` drives the salt the deterministic orders break ties with and the
/// draws the sampling orders make; one seed gives one decomposition. A
/// sampling order's weight must have one entry per vertex of `graph`.
///
/// `soft_budget` bounds construction: past it the elimination falls back to
/// its cheaper stale-heap path, and past twice it the elimination bails to a
/// path decomposition of what is left, so a decomposition is always
/// produced. `None` runs to completion.
pub fn elimination_td(
    graph: &crate::Graph,
    order: Config<'_>,
    seed: u64,
    soft_budget: Option<Duration>,
) -> crate::TreeDecomposition {
    let prebuilt = width_opt::prebuild(graph.num_vertices, &graph.edges);
    let start = crate::meter::now();
    let soft = soft_budget.map(|b| start + b);
    let hard = soft_budget.map(|b| start + b.saturating_mul(2));
    width_opt::run_config_prebuilt(
        &prebuilt,
        width_opt::RunSpec {
            config: order,
            seed,
            stop: ElimStop {
                deadline: soft,
                hard_deadline: hard,
                width_bound: None,
            },
            // Always produce a valid TD.
            force_emit: true,
        },
    )
    .td
}
