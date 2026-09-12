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

/// A vertex the search has already numbered. It takes no further part.
const NUMBERED: u8 = 1;
/// A vertex the current step's reach has already visited.
const REACHED: u8 = 2;

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
/// Returns `None` when `hard_deadline` passed before the search finished. Both
/// reaches read the deadline on the pacer's stride, which counts the scanning
/// the search charges, so a single step over a dense graph is interrupted
/// part-way rather than run to its end: the plain search scans every unnumbered
/// vertex per step and one MCS-M step walks the whole graph in the worst case,
/// and a caller with milliseconds left cannot afford either.
pub(crate) fn cardinality_search(
    adjacency: &[Vec<u32>],
    reach: Reach,
    hard_deadline: Option<Instant>,
) -> Option<Vec<u32>> {
    let n = adjacency.len();
    // Counts and marks apart, one entry per vertex each: the step order reads
    // the counts alone, as one flat scan that a numbered vertex does not
    // interrupt, since numbering a vertex zeroes its count. The reach marks
    // are cleared after every step, so between steps a mark is either empty
    // or [`NUMBERED`].
    let mut counts = vec![0u32; n];
    let mut states = vec![0u8; n];
    // The highest count in each run of 64 vertices, so a step scans the runs
    // and then one run rather than every vertex. A count only rises, except
    // to zero when its vertex is numbered, and that is the one time a run's
    // maximum is counted again.
    let mut run_max = vec![0u32; n.div_ceil(64)];
    // The lowest unnumbered vertex, which is what a step takes when no
    // unnumbered vertex counts anything yet.
    let mut lowest = 0usize;
    let mut selected: Vec<u32> = Vec::with_capacity(n);
    // The vertices the current step marked, and the buckets the path search
    // walks, both kept across steps and cleared after each one so the search
    // allocates once.
    let mut touched: Vec<u32> = Vec::new();
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); n + 1];
    let mut raised: Vec<u32> = Vec::new();
    let mut pacer = DeadlinePacer::new();

    for _ in 0..n {
        // The step's own scan of every vertex, charged before it runs so the
        // pacer counts a search that reaches nothing else.
        crate::meter::charge(n as u64);
        if pacer.due() && expired(hard_deadline) {
            return None;
        }
        debug_assert!(
            states.iter().all(|&state| state & REACHED == 0),
            "a step cleared its own marks"
        );
        while states[lowest] != 0 {
            lowest += 1;
        }
        // The highest count, then the first vertex holding it: a numbered
        // vertex counts zero, so it is never that vertex unless nothing
        // unnumbered counts either, and then the lowest unnumbered one is
        // taken instead. Ties go to the smallest index either way.
        let best = run_max.iter().copied().max().unwrap_or(0);
        let chosen = if best == 0 {
            lowest
        } else {
            let run = run_max.iter().position(|&max| max == best).unwrap_or(0);
            run * 64
                + counts[run * 64..]
                    .iter()
                    .take(64)
                    .position(|&count| count == best)
                    .unwrap_or(0)
        };
        debug_assert!(chosen < n, "every step has an unnumbered vertex to take");
        debug_assert!(states[chosen] == 0 && counts[chosen] == best);
        states[chosen] = NUMBERED;
        counts[chosen] = 0;
        if best != 0 {
            let run = chosen / 64;
            run_max[run] = counts[run * 64..]
                .iter()
                .take(64)
                .copied()
                .max()
                .unwrap_or(0);
        }
        selected.push(chosen as u32);

        raised.clear();
        crate::meter::charge(adjacency[chosen].len() as u64);
        for &neighbour in &adjacency[chosen] {
            if states[neighbour as usize] == 0 {
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
            states[chosen] |= REACHED;
            touched.push(chosen as u32);
            for &neighbour in &raised {
                states[neighbour as usize] |= REACHED;
                let count = counts[neighbour as usize] as usize;
                touched.push(neighbour);
                buckets[count].push(neighbour);
            }
            for level in 0..=n {
                while let Some(interior) = buckets[level].pop() {
                    crate::meter::charge(adjacency[interior as usize].len() as u64);
                    if pacer.due() && expired(hard_deadline) {
                        return None;
                    }
                    for &next in &adjacency[interior as usize] {
                        // Numbered and reached are both disqualifying, so the
                        // step reads one byte and tests it once.
                        let state = &mut states[next as usize];
                        if *state != 0 {
                            continue;
                        }
                        *state = REACHED;
                        let count = counts[next as usize] as usize;
                        touched.push(next);
                        if count > level {
                            raised.push(next);
                            buckets[count].push(next);
                        } else {
                            buckets[level].push(next);
                        }
                    }
                }
            }
            for &vertex in &touched {
                states[vertex as usize] &= !REACHED;
            }
        }
        for &vertex in &raised {
            let count = counts[vertex as usize] + 1;
            counts[vertex as usize] = count;
            let run = &mut run_max[vertex as usize / 64];
            *run = (*run).max(count);
        }
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
