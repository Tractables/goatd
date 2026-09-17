//! Refine a decomposition by splitting it around FlowCutter separators.
//!
//! Separator replacements project the current decomposition onto both sides and
//! glue the projections at a separator bag. A region whose graph is
//! disconnected is split at its components instead, with no separator and no
//! bag between the two sides. The whole-pass wrapper accepts by
//! `(treewidth, total bag size)`; sessions leave acceptance to the caller.

use std::time::{Duration, Instant};

use super::TreeDecomposition;
use super::ops::{disjoint_union, glue_at_separator, project_td_keeping_global_ids};
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
/// Above this number of active vertices, the search itself is stoppable — the
/// driver reads the deadline before every step — but the setup each region pays
/// first is not: building the region's graph and testing it for connectivity
/// charges nothing and cannot be interrupted, and on a region this size that
/// setup alone overruns the deadline. Above this gate, skip refinement and
/// return the input decomposition unchanged.
const MAX_VERTICES_FOR_REFINE: usize = 100_000;

/// Refine `td` by finding FlowCutter separators in `graph`, projecting `td`
/// onto each side of each separator, and recursing while the
/// refinement strictly improves `(width, total_bag_size)`. A region whose
/// graph is disconnected has no separator to find, so it is split at its
/// largest component instead; that split divides the bags it starts from
/// without adding to any of them, and is taken where it leaves
/// `(width, total_bag_size)` as it was.
///
/// `budget` is checked between separator searches and inside their search loops;
/// graph setup, cutter advances and decomposition reconstruction are indivisible.
/// It uses the construction clock, including charged work when the meter is armed;
/// it also arms a gate that skips subgraphs over 100 000 vertices, where the
/// uninterruptible setup a region pays before its search overruns the deadline
/// on its own. The result is never worse than `td` under
/// `(width, total_bag_size)`.
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

/// Put the two sides of a split back together: at the separator they share,
/// or side by side where the split ran along a component boundary and there is
/// no separator to share.
fn join_sides(
    left: TreeDecomposition,
    right: TreeDecomposition,
    separator: &[u32],
) -> Option<TreeDecomposition> {
    if separator.is_empty() {
        disjoint_union(left, right)
    } else {
        glue_at_separator(left, right, separator)
    }
}

/// The largest connected component of `graph` and the vertices outside it,
/// both in ascending order, or `None` where `graph` is connected.
///
/// The separator search declines a disconnected region, so the session splits
/// it here instead. The largest component goes first because the recursion
/// depth limit caps how many of these splits one branch can make, and the
/// largest component is where the search has the most to find.
fn largest_component_split(graph: &Graph) -> Option<(Vec<u32>, Vec<u32>)> {
    let num_vertices = graph.num_vertices() as usize;
    if num_vertices == 0 {
        return None;
    }

    // One flat adjacency rather than a `Vec` per vertex: a region can be the
    // whole graph, and the per-vertex headers alone would cost more than the
    // flood fill that follows.
    let mut offsets = vec![0usize; num_vertices + 1];
    for &(u, v) in graph.edges() {
        offsets[u as usize + 1] += 1;
        offsets[v as usize + 1] += 1;
    }
    for vertex in 0..num_vertices {
        offsets[vertex + 1] += offsets[vertex];
    }
    let mut neighbours = vec![0u32; offsets[num_vertices]];
    let mut cursor = offsets[..num_vertices].to_vec();
    for &(u, v) in graph.edges() {
        neighbours[cursor[u as usize]] = v;
        cursor[u as usize] += 1;
        neighbours[cursor[v as usize]] = u;
        cursor[v as usize] += 1;
    }

    const UNASSIGNED: u32 = u32::MAX;
    let mut component_of = vec![UNASSIGNED; num_vertices];
    let mut components = 0u32;
    let mut largest = 0u32;
    let mut largest_size = 0usize;
    let mut stack: Vec<u32> = Vec::new();
    for start in 0..num_vertices {
        if component_of[start] != UNASSIGNED {
            continue;
        }
        let component = components;
        components += 1;
        let mut size = 0usize;
        component_of[start] = component;
        stack.push(start as u32);
        while let Some(vertex) = stack.pop() {
            size += 1;
            let row = offsets[vertex as usize]..offsets[vertex as usize + 1];
            for &neighbour in &neighbours[row] {
                if component_of[neighbour as usize] == UNASSIGNED {
                    component_of[neighbour as usize] = component;
                    stack.push(neighbour);
                }
            }
        }
        if size > largest_size {
            largest = component;
            largest_size = size;
        }
    }
    if components < 2 {
        return None;
    }

    let mut inside = Vec::with_capacity(largest_size);
    let mut outside = Vec::with_capacity(num_vertices - largest_size);
    for (vertex, &component) in component_of.iter().enumerate() {
        if component == largest {
            inside.push(vertex as u32);
        } else {
            outside.push(vertex as u32);
        }
    }
    Some((inside, outside))
}
