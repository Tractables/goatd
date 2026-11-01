//! One hypergraph-coarsening level: match vertices in pairs, contract each pair into a
//! coarse vertex, rebuild the hypergraph.
//!
//! Contraction is by matching, so a level can at best halve the vertex count;
//! `multilevel_pass` calls this in a loop until it declines. Coarsening also
//! shrinks the hyperedges themselves — pins that land in the same coarse vertex
//! merge, a hyperedge left with one pin disappears, and hyperedges that end up
//! with identical pin sets become one with their weights summed.

use super::model::{Hypergraph, merge_sorted_hyperedges};
use crate::partition::common::{
    coarse_vertex_weights, matching_order, record_match, select_best_neighbor, shrank_enough,
};
use crate::rng::Xorshift64;

pub(super) struct CoarseningLevel {
    pub(super) hg: Hypergraph,
    pub(super) mapping: Vec<u32>,
}

/// If `part` is provided, preferentially match vertices in the same partition (V-cycle).
pub(super) fn coarsen_one_level(
    hg: &Hypergraph,
    min_vertices: usize,
    rng: &mut Xorshift64,
    part: Option<&[u8]>,
) -> Option<CoarseningLevel> {
    let n = hg.vertex_count;
    if n <= min_vertices {
        return None;
    }

    // A vertex's degree here is the number of hyperedges it is a pin of.
    let perm = matching_order(n, |v| hg.vertex_degree(v), &hg.vertex_weights, rng);

    let mut match_of = vec![None; n];
    let mut coarse_id: Vec<u32> = vec![0; n];
    let mut num_coarse: u32 = 0;

    // Shared weight per candidate, indexed by vertex and cleared over
    // `candidates` after each match: a map per vertex per level is one
    // allocation for every vertex of every level of every restart.
    let mut connectivity: Vec<u32> = vec![0; n];
    let mut candidates: Vec<u32> = Vec::new();

    for &v in &perm {
        if match_of[v].is_some() {
            continue;
        }

        // Candidates are ranked by the total weight of the hyperedges they
        // share with `v`. This makes an explicitly weighted hyperedge, including
        // canonicalized repeats whose weights were added, influence coarsening
        // by the same amount that it influences the cut objective.
        candidates.clear();
        for &hei in hg.vertex_hyperedges(v) {
            let weight = hg.hyperedge_weights[hei as usize];
            for &u in hg.charged_hyperedge_pins(hei as usize) {
                let u = u as usize;
                if u != v && match_of[u].is_none() {
                    // Hyperedge weights are positive, so a candidate's entry is
                    // zero exactly until it is first reached.
                    if connectivity[u] == 0 {
                        candidates.push(u as u32);
                    }
                    connectivity[u] = connectivity[u]
                        .checked_add(weight)
                        .expect("validated total hyperedge weight fits in u32");
                }
            }
        }

        let best_neighbor = select_best_neighbor(
            v,
            part,
            candidates.iter().map(|&nb| (nb, connectivity[nb as usize])),
        );
        for &nb in &candidates {
            connectivity[nb as usize] = 0;
        }

        record_match(
            v,
            best_neighbor,
            &mut match_of,
            &mut coarse_id,
            &mut num_coarse,
        );
    }

    let nc = num_coarse as usize;
    if !shrank_enough(n, nc) {
        return None;
    }

    let coarse_vertex_weights = coarse_vertex_weights(nc, &coarse_id, &hg.vertex_weights);

    let num_hyperedges = hg.num_hyperedges();
    let mut weighted_hyperedges: Vec<(Vec<u32>, u32)> = Vec::with_capacity(num_hyperedges);
    for hyperedge in 0..num_hyperedges {
        let pins = hg.charged_hyperedge_pins(hyperedge);
        let mut coarse_pins: Vec<u32> = pins.iter().map(|&v| coarse_id[v as usize]).collect();
        coarse_pins.sort_unstable();
        coarse_pins.dedup();
        if coarse_pins.len() >= 2 {
            weighted_hyperedges.push((coarse_pins, hg.hyperedge_weights[hyperedge]));
        }
    }

    // Sorting by pin set brings identical coarse hyperedges together so
    // `merge_sorted_hyperedges` can merge them by summing weights: two
    // hyperedges that this contraction made indistinguishable are one
    // hyperedge of their combined weight from here up.
    let (coarse_hyperedges, coarse_hyperedge_weights) =
        merge_sorted_hyperedges(weighted_hyperedges)
            .expect("coarsening preserves the validated total hyperedge weight");

    let mut coarse_hg =
        Hypergraph::from_hyperedges(nc, &coarse_hyperedges, Some(&coarse_hyperedge_weights));
    coarse_hg.vertex_weights = coarse_vertex_weights;

    Some(CoarseningLevel {
        hg: coarse_hg,
        mapping: coarse_id,
    })
}
