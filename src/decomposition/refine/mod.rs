//! Refine a decomposition by splitting it around FlowCutter separators.
//!
//! Each accepted step projects the current decomposition onto both sides of a
//! separator, glues the projections at a separator bag, and strictly improves
//! `(treewidth, total bag size)`.

use std::time::{Duration, Instant};

use super::TreeDecomposition;
use super::ops::{glue_at_separator, project_td_keeping_global_ids};
use crate::deadline::expired;
use crate::flowcutter::separator::{self, Budget as SeparatorBudget};
use crate::{Error, Graph};

#[cfg(test)]
mod tests;

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
/// `budget` bounds the refinement between separator searches, never inside one;
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
    let all_vertices: Vec<u32> = (0..graph.num_vertices).collect();
    Ok(refine_inner(td, &all_vertices, graph, 0, deadline))
}

fn refine_inner(
    td: TreeDecomposition,
    all_vertices: &[u32],
    graph: &Graph,
    depth: u32,
    deadline: Option<Instant>,
) -> TreeDecomposition {
    if depth >= MAX_RECURSION_DEPTH {
        return td;
    }
    if all_vertices.len() < MIN_REFINEMENT_VERTICES {
        return td;
    }
    // Large-graph gate: only applies when a deadline is set. A caller with no
    // deadline passes `None`, which bypasses the gate.
    if deadline.is_some() && all_vertices.len() > MAX_VERTICES_FOR_REFINE {
        return td;
    }
    // Deadline guard: once the shared hard deadline passes, return the
    // current decomposition unchanged so the caller always has a valid result.
    if expired(deadline) {
        return td;
    }

    let local_graph = graph
        .induced_subgraph(all_vertices)
        .expect("refinement keeps unique in-range graph vertices");
    if local_graph.edges().is_empty() {
        return td;
    }

    let flowcutter_steps = (REFINEMENT_STEPS_PER_VERTEX * all_vertices.len() as u64)
        .clamp(MIN_REFINEMENT_STEPS, MAX_REFINEMENT_STEPS);

    let Some(sep_result) = separator::find(
        &local_graph,
        SeparatorBudget::new(flowcutter_steps, FLOWCUTTER_REFINEMENT_ITERATIONS),
    )
    .expect("refinement uses positive separator limits") else {
        return td;
    };
    // Re-check the deadline after FlowCutter: on large graphs one search can
    // consume most of the remaining budget, so projection + glue + recursion
    // would overrun. Prefer returning the input decomposition over a partially built
    // refinement that won't finish in time.
    if expired(deadline) {
        return td;
    }

    let sep_global = to_global_vertices(sep_result.vertices(), all_vertices);
    let side_a_global = to_global_vertices(sep_result.side_a(), all_vertices);
    let side_b_global = to_global_vertices(sep_result.side_b(), all_vertices);

    let keep_a: Vec<u32> = side_a_global
        .iter()
        .chain(sep_global.iter())
        .copied()
        .collect();
    let keep_b: Vec<u32> = side_b_global
        .iter()
        .chain(sep_global.iter())
        .copied()
        .collect();

    let Some(td_a) = project_td_keeping_global_ids(&td, &keep_a) else {
        return td;
    };
    let Some(td_b) = project_td_keeping_global_ids(&td, &keep_b) else {
        return td;
    };
    let Some(glued) = glue_at_separator(td_a.clone(), td_b.clone(), &sep_global) else {
        return td;
    };

    if glued.quality_key() >= td.quality_key() {
        return td;
    }

    let td_a_refined = refine_inner(td_a, &keep_a, graph, depth + 1, deadline);
    let td_b_refined = refine_inner(td_b, &keep_b, graph, depth + 1, deadline);

    match glue_at_separator(td_a_refined, td_b_refined, &sep_global) {
        Some(refined) if refined.quality_key() <= glued.quality_key() => refined,
        Some(_) | None => glued,
    }
}

fn to_global_vertices(local: &[u32], local_to_global: &[u32]) -> Vec<u32> {
    local
        .iter()
        .map(|&vertex| local_to_global[vertex as usize])
        .collect()
}
