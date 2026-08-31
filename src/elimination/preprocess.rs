//! Safe reduction rules for tree-decomposition preprocessing.
//!
//! Five fixed-point rules:
//!   - **islet**: a degree-0 vertex can be eliminated with bag `{v}`.
//!   - **twig**: a degree-1 vertex can be eliminated; its bag is {v, u}.
//!   - **series**: a degree-2 vertex with non-adjacent neighbours — add the
//!     fill edge, then eliminate; bag = {v, a, b}.
//!   - **simplicial**: a vertex whose live neighbours form a clique —
//!     eliminate with zero fill; bag = {v} ∪ N(v).
//!   - **almost-simplicial** (Bodlaender): a vertex `v` whose live neighbours
//!     form a clique except for one missing edge, *and* whose degree satisfies
//!     `deg(v) ≤ LB` for some valid lower bound `LB` on treewidth — here
//!     `LB` includes the residual's minimum degree and the largest degree of a
//!     simplicial elimination so far. Adds one fill edge then eliminates; bag
//!     size = `deg(v)+1 ≤ LB+1`.
//!
//! Reattaching the recorded bags to a decomposition of the residual produces
//! a valid decomposition of the original graph. The widest recorded bag
//! captures the width contributed by removed vertices.

use super::execution::ElimSteps;
use super::graph::EliminationGraph;

/// Output of preprocessing. `Clone` so a single preprocess result can be
/// reused across multiple orders in the portfolio — preprocessing is
/// deterministic (no salt/seed) so sharing is safe.
#[derive(Clone)]
pub(crate) struct Reduced {
    /// Reduced graph (still holds inactive slots for eliminated vertices).
    pub graph: EliminationGraph,
    /// The eliminations the rules already did, which a run over `graph`
    /// continues from.
    pub prefix: ElimSteps,
}

pub(crate) fn preprocess(mut graph: EliminationGraph) -> Reduced {
    let mut prefix = ElimSteps::default();
    // Running lower bound on tw(G), maintained from residual minimum degree
    // and simplicial/series eliminations; gates the almost-simplicial rule.
    let mut treewidth_lower_bound = 0usize;

    loop {
        // Exhaust degree-zero and degree-one vertices before any rule that can
        // emit a wider bag. One index-ordered twig scan is insufficient: a
        // high-index leaf can expose a lower-index leaf behind the cursor.
        // Reaching minimum degree two also establishes that any non-empty
        // residual component is not a forest before the series rule runs.
        let mut fired = peel_low_degree(&mut graph, &mut prefix);
        if let Some(minimum_degree) = minimum_active_degree(&graph) {
            treewidth_lower_bound = treewidth_lower_bound.max(minimum_degree);
        }
        fired |= eliminate_series_vertices(&mut graph, &mut prefix, &mut treewidth_lower_bound);
        fired |= eliminate_simplicial_vertices(&mut graph, &mut prefix, &mut treewidth_lower_bound);
        fired |=
            eliminate_almost_simplicial_vertices(&mut graph, &mut prefix, treewidth_lower_bound);

        if !fired {
            break;
        }
    }

    Reduced { graph, prefix }
}

/// The minimum degree of the current non-empty residual is a treewidth lower
/// bound: every graph of treewidth `k` has a vertex of degree at most `k`.
fn minimum_active_degree(graph: &EliminationGraph) -> Option<usize> {
    (0..graph.len())
        .filter(|&vertex| graph.active[vertex])
        .map(|vertex| graph.degree(vertex as u32))
        .min()
}

/// Eliminate one vertex and record its bag before the graph changes.
fn eliminate_and_record(
    graph: &mut EliminationGraph,
    prefix: &mut ElimSteps,
    vertex: u32,
) -> usize {
    let neighbours = graph.live_neighbours(vertex);
    let degree = neighbours.len();
    let mut bag = Vec::with_capacity(degree + 1);
    bag.push(vertex);
    bag.extend(neighbours);
    graph.eliminate(vertex);
    prefix.sink().record(vertex, bag);
    degree
}

fn eliminate_series_vertices(
    graph: &mut EliminationGraph,
    prefix: &mut ElimSteps,
    treewidth_lower_bound: &mut usize,
) -> bool {
    let mut fired = false;
    for vertex in 0..graph.len() as u32 {
        if !graph.active[vertex as usize] || graph.degree(vertex) != 2 {
            continue;
        }
        let neighbours = graph.live_neighbours(vertex);
        if !graph.contains_edge(neighbours[0], neighbours[1]) {
            eliminate_and_record(graph, prefix, vertex);
            *treewidth_lower_bound = (*treewidth_lower_bound).max(2);
            fired = true;
        }
    }
    fired
}

fn eliminate_simplicial_vertices(
    graph: &mut EliminationGraph,
    prefix: &mut ElimSteps,
    treewidth_lower_bound: &mut usize,
) -> bool {
    let mut fired = false;
    for vertex in 0..graph.len() as u32 {
        if graph.active[vertex as usize] && graph.degree(vertex) >= 2 && graph.is_simplicial(vertex)
        {
            let degree = eliminate_and_record(graph, prefix, vertex);
            *treewidth_lower_bound = (*treewidth_lower_bound).max(degree);
            fired = true;
        }
    }
    fired
}

/// Apply the almost-simplicial rule where the known lower bound makes it safe.
fn eliminate_almost_simplicial_vertices(
    graph: &mut EliminationGraph,
    prefix: &mut ElimSteps,
    treewidth_lower_bound: usize,
) -> bool {
    if treewidth_lower_bound < 2 {
        return false;
    }

    let mut fired = false;
    for vertex in 0..graph.len() as u32 {
        if !graph.active[vertex as usize] {
            continue;
        }
        let degree = graph.degree(vertex);
        if degree < 2 || degree > treewidth_lower_bound {
            continue;
        }
        let Some((left, right)) = almost_simplicial_nonedge(graph, vertex) else {
            continue;
        };
        graph.add_edge(left, right);
        eliminate_and_record(graph, prefix, vertex);
        fired = true;
    }
    fired
}

/// Eliminate degree-zero and degree-one vertices to a fixed point.
fn peel_low_degree(graph: &mut EliminationGraph, prefix: &mut ElimSteps) -> bool {
    let mut fired_any = false;
    loop {
        let mut fired = false;
        for v in 0..graph.len() {
            if !graph.active[v] {
                continue;
            }
            match graph.degree(v as u32) {
                0 => {
                    graph.active[v] = false;
                    graph.num_active -= 1;
                    prefix.sink().record(v as u32, vec![v as u32]);
                    fired = true;
                }
                1 => {
                    let neighbour = graph.live_neighbours(v as u32)[0];
                    graph.remove_without_fill(v as u32);
                    prefix.sink().record(v as u32, vec![v as u32, neighbour]);
                    fired = true;
                }
                _ => {}
            }
        }
        if !fired {
            return fired_any;
        }
        fired_any = true;
    }
}

/// If v's live neighbourhood is a clique except for exactly one missing edge,
/// return that edge `(a, b)`. Otherwise return `None` (either simplicial,
/// caught by the simplicial pass earlier, or ≥ 2 missing edges).
fn almost_simplicial_nonedge(graph: &EliminationGraph, v: u32) -> Option<(u32, u32)> {
    let nbrs = graph.live_neighbours(v);
    let mut miss: Option<(u32, u32)> = None;
    for i in 0..nbrs.len() {
        for j in (i + 1)..nbrs.len() {
            if !graph.contains_edge(nbrs[i], nbrs[j]) {
                if miss.is_some() {
                    return None;
                }
                miss = Some((nbrs[i], nbrs[j]));
            }
        }
    }
    miss
}
