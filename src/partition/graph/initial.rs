//! Partitions the coarsest graph from nothing, the one phase of the sweep with
//! no partition to start from.
//!
//! Two generators, several restarts each: graph growing from a random seed, and
//! random bisections cleaned up by FM. The best edge cut among them is what
//! uncoarsening starts from, so this is also the only place the module picks
//! between whole partitions rather than single moves.

use super::csr::CsrGraph;
use super::refine_fm::{FmScratch, refine_level};
use crate::partition::common::{BisectionStop, random_bisection};
use crate::rng::Xorshift64;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Grows side 0 outward from `seed` until it holds half the vertex weight;
/// everything else lands on side 1.
///
/// The vertex added each round is the one with the largest gain, ties going to
/// the lowest index. Gains only ever rise (an edge is added at `2 * w`), so a
/// max-heap keyed by `(gain, Reverse(v))` picks the same vertex a scan of all
/// of them would: an entry whose gain no longer matches the vertex's is stale
/// and a larger one for the same vertex is still in the heap. It reads `stop`
/// as it goes and leaves the rest of the vertices on side 1 when the cutoff
/// passes, which the caller discards.
pub(super) fn greedy_graph_growing(
    graph: &CsrGraph,
    seed: usize,
    stop: &mut BisectionStop,
) -> Vec<u8> {
    let n = graph.num_vertices();
    let total_weight: u32 = graph.vertex_weights.iter().sum();
    let target = total_weight / 2;

    let mut part = vec![1u8; n];
    let mut in_set = vec![false; n];
    let mut gain: Vec<i64> = vec![0; n];

    part[seed] = 0;
    in_set[seed] = true;
    let mut set_weight = graph.vertex_weights[seed];

    let s_start = graph.offsets[seed] as usize;
    let s_end = graph.offsets[seed + 1] as usize;
    for (&nb, &w) in graph.neighbors[s_start..s_end]
        .iter()
        .zip(&graph.edge_weights[s_start..s_end])
    {
        gain[nb as usize] += w as i64;
    }

    // Every vertex outside the set is a candidate from the start, including the
    // ones no edge has touched yet, so all of them go in.
    let mut heap: BinaryHeap<(i64, Reverse<usize>)> = (0..n)
        .filter(|&v| v != seed)
        .map(|v| (gain[v], Reverse(v)))
        .collect();

    while set_weight < target {
        // Still charged a pass over the vertices per vertex added, which is
        // what the heap replaced. The cutoff below reads the meter, so a
        // different charge would move where a budgeted bisection stops.
        crate::meter::charge(n as u64);
        if stop.reached() {
            break;
        }
        let mut best_v = None;
        while let Some((g, Reverse(v))) = heap.pop() {
            if !in_set[v] && g == gain[v] {
                best_v = Some(v);
                break;
            }
        }

        let Some(v) = best_v else {
            break;
        };
        part[v] = 0;
        in_set[v] = true;
        set_weight += graph.vertex_weights[v];

        let start = graph.offsets[v] as usize;
        let end = graph.offsets[v + 1] as usize;
        for (&nb, &w) in graph.neighbors[start..end]
            .iter()
            .zip(&graph.edge_weights[start..end])
        {
            let nb = nb as usize;
            if !in_set[nb] {
                // `2 * w`: the edge leaves the not-in-set sum and enters the
                // in-set sum, so the difference between them moves by twice its
                // weight. The seed's own edges above were added at `w`, so an
                // edge to the seed counts half of what every later edge does,
                // and a candidate's edges to vertices still outside the set are
                // never subtracted at all — the score ranks candidates but is
                // not the cut reduction the textbook version tracks. See "Where
                // the two bisectors differ" in the shared partition bookkeeping.
                gain[nb] += 2 * w as i64;
                if w > 0 {
                    heap.push((gain[nb], Reverse(nb)));
                }
            }
        }
    }

    part
}

/// Summed weight of the cut edges, each counted once: the scan visits only
/// side-0 vertices, so an edge is reached from its side-0 endpoint alone.
pub(super) fn edge_cut(graph: &CsrGraph, part: &[u8]) -> u64 {
    // A full pass, charged as one: the scan skips side-1 vertices, so what it
    // touches is bounded by a pass rather than equal to it.
    crate::meter::charge(graph.pass_units());
    let mut cut: u64 = 0;
    for v in 0..graph.num_vertices() {
        if part[v] == 0 {
            let start = graph.offsets[v] as usize;
            let end = graph.offsets[v + 1] as usize;
            for (&nb, &w) in graph.neighbors[start..end]
                .iter()
                .zip(&graph.edge_weights[start..end])
            {
                if part[nb as usize] != 0 {
                    cut += w as u64;
                }
            }
        }
    }
    cut
}

pub(super) fn initial_partition(
    graph: &CsrGraph,
    rng: &mut Xorshift64,
    max_imbalance: f64,
    scratch: &mut FmScratch,
    stop: &mut BisectionStop,
) -> Vec<u8> {
    let n = graph.num_vertices();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![0];
    }

    let mut best_part = Vec::new();
    let mut best_cut = u64::MAX;

    // Restart count: see "Where the two bisectors differ" in the shared
    // partition bookkeeping.
    for _ in 0..4.min(n) {
        let seed = (rng.next_u64() as usize) % n;
        let part = greedy_graph_growing(graph, seed, stop);
        if stop.stopped() {
            return part;
        }
        let cut = edge_cut(graph, &part);
        if cut < best_cut {
            best_cut = cut;
            best_part = part;
        }
    }

    // Random starts get an FM pass before being scored; grown ones are scored
    // as produced.
    for _ in 0..4.min(n) {
        let mut part = random_bisection(&graph.vertex_weights, rng);
        refine_level(graph, &mut part, max_imbalance, scratch, stop);
        if stop.stopped() {
            return part;
        }
        let cut = edge_cut(graph, &part);
        if cut < best_cut {
            best_cut = cut;
            best_part = part;
        }
    }

    best_part
}
