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

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::Instant;

use super::execution::{
    Cutoff, DeadlinePacer, ElimExit, ElimSink, ElimStop, eliminate_in_order, residual_edges,
};
use super::graph::EliminationGraph;
use crate::deadline::expired;
use crate::rng::{SEED_OFFSET, Xorshift64};

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

/// Which vertex a step takes when several have the same count.
#[derive(Clone, Copy)]
pub(crate) enum Ties<'a> {
    /// The smallest vertex index, so one graph gives one order.
    SmallestIndex,
    /// The one a permutation of the vertices puts first. `rank[v]` is the
    /// vertex's place in the permutation and `of_rank[rank[v]] == v`; a
    /// caller that draws the permutation from a seed gets a different order
    /// per seed out of the same search.
    Ranked {
        /// Each vertex's place in the permutation.
        rank: &'a [u32],
        /// The vertex at each place, the inverse of `rank`.
        of_rank: &'a [u32],
    },
}

/// Run a cardinality search over `adjacency` and return the vertices in the
/// order the search numbered them, highest number first.
///
/// `ties` decides which of several vertices with the same count a step takes.
/// The elimination order is this sequence reversed: the last vertex numbered
/// is eliminated first.
///
/// Returns `None` when `hard_deadline` passed before the search finished. Both
/// reaches read the deadline on the pacer's stride, which counts the work the
/// search charges, so a single step over a dense graph is interrupted part-way
/// rather than run to its end: one MCS-M step walks the whole graph in the
/// worst case, and a caller with milliseconds left cannot afford it.
pub(crate) fn cardinality_search(
    adjacency: &[Vec<u32>],
    reach: Reach,
    ties: Ties<'_>,
    hard_deadline: Option<Instant>,
) -> Option<Vec<u32>> {
    let n = adjacency.len();
    let mut numbered = vec![false; n];
    let mut count = vec![0u32; n];
    let mut selected: Vec<u32> = Vec::with_capacity(n);
    // Reached-vertex marks and the buckets the path search walks, both kept
    // across steps and cleared after each one so the search allocates once.
    // Only MCS-M walks paths, and at the sizes the plain search now runs on
    // the bucket row alone is tens of megabytes, so neither is allocated for a
    // reach that does not use them.
    let paths = reach == Reach::LowerPaths;
    let mut reached = vec![false; if paths { n } else { 0 }];
    let mut touched: Vec<u32> = Vec::new();
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); if paths { n + 1 } else { 0 }];
    let mut raised: Vec<u32> = Vec::new();
    // The step's pick, off a max-heap keyed on the count and, at equal counts,
    // on the smallest index — the vertex a scan of the whole graph would have
    // taken, without the scan. A count only ever rises, so an entry is stale
    // exactly when it sits below its vertex's current count, and the entry
    // carrying that current count is always in the heap; a stale one is
    // dropped when it surfaces rather than gone looking for.
    // The heap is keyed on the tie key rather than the vertex, so the two tie
    // rules are one heap; `vertex_at` reads the key back as a vertex.
    let key_of = |vertex: u32| match ties {
        Ties::SmallestIndex => vertex,
        Ties::Ranked { rank, .. } => rank[vertex as usize],
    };
    let vertex_at = |key: u32| match ties {
        Ties::SmallestIndex => key,
        Ties::Ranked { of_rank, .. } => of_rank[key as usize],
    };
    let mut queue: BinaryHeap<(u32, Reverse<u32>)> =
        (0..n as u32).map(|key| (0, Reverse(key))).collect();
    // Ordering the vertices into a heap is linear, and it is the one part of
    // the search a deadline cannot interrupt.
    crate::meter::charge(n as u64);
    let mut pacer = DeadlinePacer::new();

    for _ in 0..n {
        if pacer.due() && expired(hard_deadline) {
            return None;
        }
        let mut chosen = usize::MAX;
        while let Some((level, Reverse(key))) = queue.pop() {
            crate::meter::charge(sift_units(queue.len()));
            let index = vertex_at(key) as usize;
            if numbered[index] || count[index] != level {
                continue;
            }
            chosen = index;
            break;
        }
        debug_assert!(chosen < n, "every step has an unnumbered vertex to take");
        numbered[chosen] = true;
        selected.push(chosen as u32);

        raised.clear();
        crate::meter::charge(adjacency[chosen].len() as u64);
        for &neighbour in &adjacency[chosen] {
            if !numbered[neighbour as usize] {
                raised.push(neighbour);
            }
        }
        if paths {
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
                    crate::meter::charge(adjacency[interior as usize].len() as u64);
                    if pacer.due() && expired(hard_deadline) {
                        return None;
                    }
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
            let index = vertex as usize;
            count[index] += 1;
            crate::meter::charge(sift_units(queue.len()));
            queue.push((count[index], Reverse(key_of(vertex))));
        }
    }
    Some(selected)
}

/// What one heap operation costs, in the units the meter charges.
///
/// A push or a pop moves one entry along a root-to-leaf path, so the work is
/// the heap's depth and the whole search charges the `(n + raises) log n` it
/// actually does. The floor of one keeps a step on an empty heap counted.
#[inline]
fn sift_units(len: usize) -> u64 {
    (usize::BITS - len.leading_zeros()).max(1) as u64
}

/// Number the active residual with a cardinality search of the given reach and
/// eliminate along the ordering that comes out.
///
/// With [`Reach::LowerPaths`] the elimination fills the residual to a minimal
/// triangulation. With [`Reach::Neighbours`] it adds whatever fill the plain
/// numbering happens to need, which is none when the residual is already
/// chordal.
///
/// With `tie_seed` set the search takes its tied vertices in the order of a
/// permutation drawn from that seed instead of by index, so the same residual
/// gives a different numbering per seed. Without one the search is the
/// deterministic one and allocates nothing extra.
pub(super) fn eliminate_cardinality_search(
    graph: &mut EliminationGraph,
    reach: Reach,
    tie_seed: Option<u64>,
    mut sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    let (active, adjacency) = residual_edges(graph);
    let permutation = tie_seed.map(|seed| tie_permutation(adjacency.len(), seed));
    let ties = match &permutation {
        None => Ties::SmallestIndex,
        Some((rank, of_rank)) => Ties::Ranked { rank, of_rank },
    };
    let Some(selected) = cardinality_search(&adjacency, reach, ties, stop.hard_deadline) else {
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

/// A permutation of `n` vertices drawn from `seed`, as (rank, of_rank).
///
/// Fisher-Yates over the run's own generator, so a seed gives one permutation
/// and two seeds give unrelated ones. The pass is linear in `n` and the search
/// that follows costs a pass over the edges, so drawing it is not what decides
/// whether a restart fits.
pub(crate) fn tie_permutation(n: usize, seed: u64) -> (Vec<u32>, Vec<u32>) {
    let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
    let mut of_rank: Vec<u32> = (0..n as u32).collect();
    for place in (1..n).rev() {
        let other = (rng.next_u64() % (place as u64 + 1)) as usize;
        of_rank.swap(place, other);
    }
    let mut rank = vec![0u32; n];
    for (place, &vertex) in of_rank.iter().enumerate() {
        rank[vertex as usize] = place as u32;
    }
    crate::meter::charge(n as u64);
    (rank, of_rank)
}
