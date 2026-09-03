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
//!     `LB = max deg over simplicial eliminations so far`. Adds one fill edge
//!     then eliminates; bag size = `deg(v)+1 ≤ LB+1`.
//!
//! Reattaching the recorded bags to a decomposition of the residual produces
//! a valid decomposition of the original graph. The widest recorded bag
//! captures the width contributed by removed vertices.

use std::time::Instant;

use super::execution::{DeadlinePacer, ElimSteps};
use super::graph::EliminationGraph;
use crate::deadline::expired;

/// The clock preprocessing runs against.
///
/// The rules are fixed-point scans over the whole graph and the two clique
/// tests are the expensive part of them, so on a dense residual one pass can
/// take seconds. Reduction is optional — whatever it has not done, the
/// elimination that follows does — so it stops when the caller's soft cutoff
/// arrives and hands the rest of the graph on.
pub(crate) struct PreprocessStop {
    deadline: Option<Instant>,
    pacer: DeadlinePacer,
    stopped: bool,
}

impl PreprocessStop {
    pub(crate) fn new(deadline: Option<Instant>) -> Self {
        Self {
            deadline,
            pacer: DeadlinePacer::new(),
            stopped: false,
        }
    }

    /// Whether the cutoff has arrived, asked at most once per millisecond of
    /// charged work.
    #[inline]
    fn reached(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        if self.deadline.is_some() && self.pacer.due() && expired(self.deadline) {
            self.stopped = true;
        }
        self.stopped
    }
}

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

pub(crate) fn preprocess(graph: EliminationGraph, deadline: Option<Instant>) -> Reduced {
    let mut stop = PreprocessStop::new(deadline);
    preprocess_with_stop(graph, &mut stop)
}

fn preprocess_with_stop(mut graph: EliminationGraph, stop: &mut PreprocessStop) -> Reduced {
    let mut prefix = ElimSteps::default();
    // Running lower bound on tw(G), maintained across simplicial/series
    // eliminations; gates the almost-simplicial rule below.
    let mut treewidth_lower_bound = 0usize;

    loop {
        // Exhaust degree-zero and degree-one vertices before any rule that can
        // emit a wider bag. One index-ordered twig scan is insufficient: a
        // high-index leaf can expose a lower-index leaf behind the cursor.
        // Reaching minimum degree two also establishes that any non-empty
        // residual component is not a forest before the series rule runs.
        let mut fired = peel_low_degree(&mut graph, &mut prefix);
        fired |=
            eliminate_series_vertices(&mut graph, &mut prefix, &mut treewidth_lower_bound, stop);
        fired |= eliminate_simplicial_vertices(
            &mut graph,
            &mut prefix,
            &mut treewidth_lower_bound,
            stop,
        );
        fired |= eliminate_almost_simplicial_vertices(
            &mut graph,
            &mut prefix,
            treewidth_lower_bound,
            stop,
        );

        if !fired || stop.reached() {
            break;
        }
    }

    Reduced { graph, prefix }
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
    stop: &mut PreprocessStop,
) -> bool {
    let mut fired = false;
    for vertex in 0..graph.len() as u32 {
        if !graph.active[vertex as usize] || graph.degree(vertex) != 2 {
            continue;
        }
        if stop.reached() {
            return fired;
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
    stop: &mut PreprocessStop,
) -> bool {
    let mut fired = false;
    for vertex in 0..graph.len() as u32 {
        if graph.active[vertex as usize] && graph.degree(vertex) >= 2 && stop.reached() {
            return fired;
        }
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
    stop: &mut PreprocessStop,
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
        if stop.reached() {
            return fired;
        }
        let Some((left, right)) = graph.almost_simplicial_nonedge(vertex) else {
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
