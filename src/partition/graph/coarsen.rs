//! One graph-coarsening level: match vertices in pairs, contract each pair into a
//! coarse vertex, rebuild the CSR.
//!
//! Contraction is by matching, so a level can at best halve the vertex count;
//! `multilevel_pass` calls this in a loop until it declines. Only the coarse
//! graph and the fine-to-coarse mapping survive a level — the fine graph is
//! kept by the caller, since uncoarsening needs it back.

use super::csr::CsrGraph;
use crate::partition::common::{
    coarse_vertex_weights, matching_order, record_match, select_best_neighbor, shrank_enough,
};
use crate::rng::Xorshift64;

pub(super) struct CoarseningLevel {
    pub(super) graph: CsrGraph,
    /// mapping[fine_vertex] = coarse_vertex it was contracted into.
    pub(super) mapping: Vec<u32>,
}

/// If `part` is provided, preferentially match same-side vertices — the key
/// idea in V-cycle refinement (Karypis & Kumar 1998, Section 5.4): the coarse
/// graph then respects the partition being refined instead of cutting across
/// it. `None` is plain heavy-edge matching.
pub(super) fn coarsen_one_level(
    graph: &CsrGraph,
    min_vertices: usize,
    rng: &mut Xorshift64,
    part: Option<&[u8]>,
) -> Option<CoarseningLevel> {
    // One pass over the graph: the matching sweep, the coarse-weight fold and
    // the CSR rebuild each walk it once. Charged before the floor test below, so
    // a level that declines to coarsen is charged for a pass it did not take —
    // over-charging is the safe direction for a clock whose job is to stop a
    // build before a wall does. See [`CsrGraph::pass_units`].
    crate::meter::charge(graph.pass_units());
    let n = graph.num_vertices();
    if n <= min_vertices {
        return None;
    }

    let perm = matching_order(
        n,
        |v| graph.offsets[v + 1] - graph.offsets[v],
        &graph.vertex_weights,
        rng,
    );
    let (coarse_id, nc) = match_fine_vertices(graph, &perm, part);
    if !shrank_enough(n, nc) {
        return None;
    }
    let coarse_vwgt = coarse_vertex_weights(nc, &coarse_id, &graph.vertex_weights);
    Some(CoarseningLevel {
        graph: build_coarse_csr(graph, nc, &coarse_id, coarse_vwgt),
        mapping: coarse_id,
    })
}

fn match_fine_vertices(graph: &CsrGraph, perm: &[usize], part: Option<&[u8]>) -> (Vec<u32>, usize) {
    let n = graph.num_vertices();
    let mut match_of = vec![None; n];
    let mut coarse_id: Vec<u32> = vec![0; n];
    let mut num_coarse: u32 = 0;
    for &vtx in perm {
        if match_of[vtx].is_some() {
            continue;
        }
        let (start, end) = (graph.offsets[vtx] as usize, graph.offsets[vtx + 1] as usize);
        let neighbors = &graph.neighbors[start..end];
        let weights = &graph.edge_weights[start..end];
        let best_neighbor = select_best_neighbor(
            vtx,
            part,
            neighbors
                .iter()
                .copied()
                .zip(weights.iter().copied())
                .filter(|&(nb, _)| match_of[nb as usize].is_none()),
        );
        record_match(
            vtx,
            best_neighbor,
            &mut match_of,
            &mut coarse_id,
            &mut num_coarse,
        );
    }
    (coarse_id, num_coarse as usize)
}

fn build_coarse_csr(
    graph: &CsrGraph,
    nc: usize,
    coarse_id: &[u32],
    coarse_vwgt: Vec<u32>,
) -> CsrGraph {
    let (offsets, mut flat) = collect_coarse_edge_entries(graph, nc, coarse_id);
    let total = flat.len();

    // Sorting each cv's slice keeps CSR order deterministic. Merging the
    // parallel entries sums their weights, which is what makes heavy-edge
    // matching meaningful one level up: a coarse edge's weight is the number of
    // fine edges that would be cut by separating its two endpoints.
    //
    // `total` counts the entries before merging, so it is an upper bound on the
    // merged rows: neither output array has to grow.
    let mut coarse_offsets = Vec::with_capacity(nc + 1);
    let mut coarse_neighbors = Vec::with_capacity(total);
    let mut coarse_edge_weights = Vec::with_capacity(total);
    coarse_offsets.push(0u32);
    for v in 0..nc {
        let start = offsets[v] as usize;
        let end = offsets[v + 1] as usize;
        let slice = &mut flat[start..end];
        slice.sort_unstable_by_key(|&(nb, _)| nb);
        let mut i = 0;
        while i < slice.len() {
            let nb = slice[i].0;
            let mut wt = 0u32;
            while i < slice.len() && slice[i].0 == nb {
                wt = wt
                    .checked_add(slice[i].1)
                    .expect("a coarse edge cannot outweigh all fine edges");
                i += 1;
            }
            coarse_neighbors.push(nb);
            coarse_edge_weights.push(wt);
        }
        coarse_offsets.push(coarse_neighbors.len() as u32);
    }

    CsrGraph {
        offsets: coarse_offsets,
        neighbors: coarse_neighbors,
        vertex_weights: coarse_vwgt,
        edge_weights: coarse_edge_weights,
    }
}

fn collect_coarse_edge_entries(
    graph: &CsrGraph,
    nc: usize,
    coarse_id: &[u32],
) -> (Vec<u32>, Vec<(u32, u32)>) {
    let n = graph.num_vertices();
    // Count then fill, with `counts` reused as the per-row cursor: the coarse
    // adjacency lands contiguously in one flat buffer instead of a Vec of Vecs.
    // Entries are duplicated here (one per fine edge) and merged below.
    let mut counts: Vec<u32> = vec![0; nc];
    for v in 0..n {
        let cv = coarse_id[v] as usize;
        for &nb in graph.neighbors(v) {
            let cnb = coarse_id[nb as usize];
            if cnb as usize != cv {
                counts[cv] += 1;
            }
        }
    }
    let mut offsets: Vec<u32> = Vec::with_capacity(nc + 1);
    offsets.push(0);
    let mut acc = 0u32;
    for &c in &counts {
        acc += c;
        offsets.push(acc);
    }
    let total = acc as usize;
    let mut flat: Vec<(u32, u32)> = vec![(0, 0); total];
    let mut cur = counts; // reuse as cursor; already sized nc
    for c in cur.iter_mut() {
        *c = 0;
    }
    for v in 0..n {
        let cv = coarse_id[v] as usize;
        let start = graph.offsets[v] as usize;
        let end = graph.offsets[v + 1] as usize;
        let neighbors = &graph.neighbors[start..end];
        let weights = &graph.edge_weights[start..end];
        for (&nb, &w) in neighbors.iter().zip(weights) {
            let cnb = coarse_id[nb as usize];
            if cnb as usize != cv {
                let pos = (offsets[cv] + cur[cv]) as usize;
                flat[pos] = (cnb, w);
                cur[cv] += 1;
            }
        }
    }
    (offsets, flat)
}
