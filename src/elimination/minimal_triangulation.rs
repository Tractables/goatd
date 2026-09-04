//! Cardinality searches, and the minimal triangulation one of them builds.
//!
//! Maximum cardinality search numbers the vertices from `n` down to 1, always
//! taking one with the most numbered neighbours. On a chordal graph the numbers
//! read backwards are a perfect elimination ordering.
//!
//! MCS-M is the same search with a longer reach: a vertex counts the numbered
//! vertices it can reach along a path whose interior vertices all have a
//! smaller count than its own endpoint. Eliminating along the numbering that
//! comes out fills the graph to a minimal triangulation — one from which no
//! single added edge can be dropped and leave the graph chordal. Berry, Blair,
//! Heggernes and Peyton, "Maximum cardinality search for computing minimal
//! triangulations of graphs", Algorithmica 39(4), 2004.
//!
//! The two searches differ only in how a step collects the vertices whose count
//! goes up, so they are one function with a switch rather than two.

use std::time::Instant;

use super::execution::{
    Cutoff, DeadlinePacer, ElimExit, ElimSink, ElimStop, eliminate_in_order, residual_edges,
};
use super::graph::EliminationGraph;
use crate::deadline::expired;

/// How far a step looks for the vertices whose count it raises.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reach {
    /// The unnumbered neighbours of the chosen vertex: maximum cardinality
    /// search, which adds no edge and orders a chordal graph.
    Neighbours,
    /// Every unnumbered vertex reachable along a path whose interior vertices
    /// all count lower than the endpoint: MCS-M, whose numbering fills the
    /// graph to a minimal triangulation.
    LowerPaths,
}

/// Run a cardinality search over `adjacency` and return the vertices in the
/// order the search numbered them, highest number first.
///
/// Ties go to the smallest vertex index, so one graph gives one order. The
/// elimination order is this sequence reversed: the last vertex numbered is
/// eliminated first.
///
/// Returns `None` when `hard_deadline` passed before the search finished.
pub(crate) fn cardinality_search(
    adjacency: &[Vec<u32>],
    reach: Reach,
    hard_deadline: Option<Instant>,
) -> Option<Vec<u32>> {
    let n = adjacency.len();
    let mut numbered = vec![false; n];
    let mut count = vec![0u32; n];
    let mut selected: Vec<u32> = Vec::with_capacity(n);
    // Reached-vertex marks and the buckets the path search walks, both kept
    // across steps and cleared after each one so the search allocates once.
    let mut reached = vec![false; n];
    let mut touched: Vec<u32> = Vec::new();
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); n + 1];
    let mut raised: Vec<u32> = Vec::new();
    let mut pacer = DeadlinePacer::new();

    for _ in 0..n {
        if pacer.due() && expired(hard_deadline) {
            return None;
        }
        let mut chosen = usize::MAX;
        for vertex in 0..n {
            if !numbered[vertex] && (chosen == usize::MAX || count[vertex] > count[chosen]) {
                chosen = vertex;
            }
        }
        debug_assert!(chosen < n, "every step has an unnumbered vertex to take");
        numbered[chosen] = true;
        selected.push(chosen as u32);

        raised.clear();
        let mut scanned = adjacency[chosen].len() as u64;
        for &neighbour in &adjacency[chosen] {
            if !numbered[neighbour as usize] {
                raised.push(neighbour);
            }
        }
        if reach == Reach::LowerPaths {
            // The neighbours are the paths of length one; each of them can then
            // carry a path on to a vertex counting higher than it does.
            // Bucket `j` holds the vertices reachable through interior vertices
            // that all count at most `j`, so draining the buckets in increasing
            // `j` reaches every vertex by its cheapest path first.
            touched.clear();
            reached[chosen] = true;
            touched.push(chosen as u32);
            for &neighbour in &raised {
                reached[neighbour as usize] = true;
                touched.push(neighbour);
                buckets[count[neighbour as usize] as usize].push(neighbour);
            }
            for level in 0..=n {
                while let Some(interior) = buckets[level].pop() {
                    scanned += adjacency[interior as usize].len() as u64;
                    for &next in &adjacency[interior as usize] {
                        let index = next as usize;
                        if numbered[index] || reached[index] {
                            continue;
                        }
                        reached[index] = true;
                        touched.push(next);
                        if count[index] as usize > level {
                            raised.push(next);
                            buckets[count[index] as usize].push(next);
                        } else {
                            buckets[level].push(next);
                        }
                    }
                }
            }
            for &vertex in &touched {
                reached[vertex as usize] = false;
            }
        }
        for &vertex in &raised {
            count[vertex as usize] += 1;
        }
        crate::meter::charge(scanned.saturating_add(n as u64));
    }
    Some(selected)
}

/// Number the active residual with a cardinality search of the given reach and
/// eliminate along the ordering that comes out.
///
/// With [`Reach::LowerPaths`] the elimination fills the residual to a minimal
/// triangulation. With [`Reach::Neighbours`] it adds whatever fill the plain
/// numbering happens to need, which is none when the residual is already
/// chordal.
pub(super) fn eliminate_cardinality_search(
    graph: &mut EliminationGraph,
    reach: Reach,
    mut sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    let (active, adjacency) = residual_edges(graph);
    let Some(selected) = cardinality_search(&adjacency, reach, stop.hard_deadline) else {
        return ElimExit::DeadlineReached(Cutoff::Hard);
    };
    // The search numbers from `n` down to 1 and the numbering is a perfect
    // elimination ordering read the other way round, so the vertex numbered
    // last leaves first.
    let order = selected
        .into_iter()
        .rev()
        .map(|local| active[local as usize]);
    eliminate_in_order(graph, order, &mut sink, stop)
}
