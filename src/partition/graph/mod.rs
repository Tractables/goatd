//! Multilevel graph bisection (Karypis & Kumar, SIAM 1998): recursive-bisection
//! partitioner for `nparts=2`.
//!
//! Three phases, one per submodule: `coarsen` contracts the graph down to a few
//! dozen vertices, `initial` partitions that, `refine` improves the partition on
//! the way back up. `multilevel_pass` drives all three.
//!
//! The result is a partition, not a separator. Nested dissection converts its
//! crossing edges to a vertex separator before building an elimination order.
//!
//! Sibling of
//! [`multilevel_hypergraph_bisect`](crate::partition::multilevel_hypergraph_bisect).
//! The two share pass bookkeeping. Their gain calculations stay separate
//! because edge cuts and hyperedge cuts change differently when a vertex
//! moves. The other differences are listed in the shared bookkeeping module.

use crate::partition::Bisection;
use crate::partition::common::{
    index_split, lift_to_fine, project_to_coarse, repair_bisection, tiny_bisection,
    validate_max_imbalance,
};
use crate::rng::{Xorshift64, bisector_stream, restart_seed};
use crate::{Error, Graph};

mod coarsen;
mod csr;
mod initial;
mod refine_fm;

#[cfg(test)]
mod tests;

use coarsen::{CoarseningLevel, coarsen_one_level};
use csr::{CsrGraph, build_csr};
use initial::{edge_cut, initial_partition};
use refine_fm::{FmScratch, refine_finest_level, refine_level};

const MIN_COARSEN_SIZE: usize = 20;
const MAX_BISECTION_EDGES: usize = u32::MAX as usize / 2;

fn validate_size(num_edges: usize) -> Result<(), Error> {
    if num_edges > MAX_BISECTION_EDGES {
        return Err(Error::TooLarge(format!(
            "graph has {num_edges} edges; multilevel bisection supports at most {MAX_BISECTION_EDGES}"
        )));
    }
    Ok(())
}

/// Controls one multilevel graph bisection.
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub struct GraphBisectionConfig {
    max_imbalance: f64,
    seed: u64,
}

impl GraphBisectionConfig {
    /// Configure a bisection. `max_imbalance` is the allowed deviation from a
    /// half-and-half split, in `0.0..=0.5`; `seed` selects the deterministic
    /// RNG stream.
    pub fn new(max_imbalance: f64, seed: u64) -> Self {
        Self {
            max_imbalance,
            seed,
        }
    }
}

/// One coarsen -> partition -> uncoarsen sweep; returns 0/1 per vertex of
/// `graph`.
///
/// `part` is an existing partition of `graph` to improve rather than replace:
/// it is projected down as the levels are built, coarsening is told to keep its
/// two sides apart, and no initial partition is taken. That is the V-cycle
/// (Karypis & Kumar 1998, Section 5.4).
fn multilevel_pass(
    graph: &CsrGraph,
    part: Option<&[u8]>,
    rng: &mut Xorshift64,
    max_imbalance: f64,
    scratch: &mut FmScratch,
) -> Vec<u8> {
    let n = graph.num_vertices();

    // Pre-allocated once, reused across all levels: avoids O(L²) reallocation.
    let mut count_scratch: Vec<[u32; 2]> = Vec::with_capacity(n);
    let mut proj_scratch: Vec<u8> = Vec::with_capacity(n);

    // Coarsening. Each level contracts matched pairs, so `levels` ends up
    // ordered finest-first and `current` walks down to the coarsest graph.
    let mut levels: Vec<CoarseningLevel> = Vec::new();
    let mut current = graph;
    loop {
        // Full re-projection each level, not incremental: incremental
        // projection degrades partition quality here.
        let mut fine_part: Option<Vec<u8>> = None;
        if let Some(p) = part {
            let mut fp = p.to_vec();
            for lv in &levels {
                let nc = lv.graph.num_vertices();
                project_to_coarse(&fp, &lv.mapping, nc, &mut count_scratch, &mut proj_scratch);
                std::mem::swap(&mut fp, &mut proj_scratch);
            }
            fine_part = Some(fp);
        }
        let level = coarsen_one_level(current, MIN_COARSEN_SIZE, rng, fine_part.as_deref());

        if let Some(level) = level {
            levels.push(level);
            current = &levels.last().unwrap().graph;
        } else {
            break;
        }
    }

    // Coarsest level: either the caller's partition projected the whole way
    // down, or a fresh one grown here. This is the only point in the sweep
    // where a partition is created rather than improved.
    let mut coarse_part = if let Some(p) = part {
        let mut fine_part = p.to_vec();
        for level in &levels {
            let nc = level.graph.num_vertices();
            project_to_coarse(
                &fine_part,
                &level.mapping,
                nc,
                &mut count_scratch,
                &mut proj_scratch,
            );
            std::mem::swap(&mut fine_part, &mut proj_scratch);
        }
        fine_part
    } else {
        initial_partition(current, rng, max_imbalance, scratch)
    };

    // Coarse edges carry the summed weight of everything contracted into them,
    // so a move here is worth many fine edges and this is the cheapest place in
    // the sweep to buy cut.
    refine_level(current, &mut coarse_part, max_imbalance, scratch);

    // Uncoarsening. Each step hands every fine vertex its coarse vertex's side,
    // then refines with the freedom the finer graph exposes; only the finest
    // level pays for the localized passes on top.
    let mut result_part = coarse_part;
    for (li, level) in levels.iter().enumerate().rev() {
        lift_to_fine(&result_part, &level.mapping, &mut proj_scratch);
        std::mem::swap(&mut result_part, &mut proj_scratch);

        let fine_graph = if li > 0 { &levels[li - 1].graph } else { graph };
        if li == 0 {
            refine_finest_level(fine_graph, &mut result_part, max_imbalance, scratch);
        } else {
            refine_level(fine_graph, &mut result_part, max_imbalance, scratch);
        }
    }

    repair_bisection(result_part, max_imbalance)
}

fn multilevel_graph_bisect_once(
    graph: &CsrGraph,
    rng: &mut Xorshift64,
    max_imbalance: f64,
    scratch: &mut FmScratch,
) -> Vec<u8> {
    let mut part = multilevel_pass(graph, None, rng, max_imbalance, scratch);

    // Arbitrary tuned thresholds: more V-cycles for larger graphs, where
    // quality matters more.
    let num_vcycles = if graph.num_vertices() >= 400 {
        4
    } else if graph.num_vertices() >= 100 {
        2
    } else {
        1
    };
    // The first cycle that fails to improve ends the loop, so `num_vcycles` is
    // a ceiling rather than a count.
    for _ in 0..num_vcycles {
        let old_cut = edge_cut(graph, &part);
        let new_part = multilevel_pass(graph, Some(&part), rng, max_imbalance, scratch);
        let new_cut = edge_cut(graph, &new_part);
        if new_cut < old_cut {
            part = new_part;
        } else {
            break;
        }
    }

    part
}

/// Multilevel 2-way bisection of a simple graph: one pass, refined by V-cycles.
/// The result contains both sides for any graph with at least two vertices.
///
/// `config.max_imbalance` bounds each side at
/// `ceil((0.5 + max_imbalance) * num_vertices)`. `config.seed` selects the RNG
/// stream; nothing here reads a clock, so one seed gives one bisection.
///
/// One pass and not a best-of-N over restarts, for the reason under "Where the
/// two bisectors differ" in the shared partition bookkeeping.
///
/// # Errors
///
/// Returns an error when the imbalance is outside `0.0..=0.5` or the CSR arc
/// count does not fit in `u32`.
pub fn multilevel_graph_bisect(
    graph: &Graph,
    config: GraphBisectionConfig,
) -> Result<Bisection, Error> {
    validate_max_imbalance(config.max_imbalance, "graph-bisection")?;
    validate_size(graph.edges.len())?;
    let n = graph.num_vertices as usize;
    if let Some(part) = tiny_bisection(n) {
        return Ok(Bisection::new(part));
    }

    let graph = build_csr(n, &graph.edges);

    if graph.neighbors.is_empty() {
        return Ok(Bisection::new(index_split(n)));
    }

    let mut scratch = FmScratch::new();

    let mut rng = bisector_stream(restart_seed(config.seed, 0));
    Ok(Bisection::new(multilevel_graph_bisect_once(
        &graph,
        &mut rng,
        config.max_imbalance,
        &mut scratch,
    )))
}
