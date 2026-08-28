//! One hypergraph-coarsening level: match vertices in pairs, contract each pair into a
//! coarse vertex, rebuild the hypergraph.
//!
//! Contraction is by matching, so a level can at best halve the vertex count;
//! `multilevel_pass` calls this in a loop until it declines. Coarsening also
//! shrinks the hyperedges themselves — pins that land in the same coarse vertex
//! merge, a hyperedge left with one pin disappears, and hyperedges that end up
//! with identical pin sets become one with their weights summed.

use rustc_hash::FxHashMap;

use super::model::Hypergraph;
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
    let n = hg.num_vertices;
    if n <= min_vertices {
        return None;
    }

    // SHEM analog for hypergraphs: degree-ascending order leaves high-degree
    // hubs to match last, with connected partners.
    let mut perm: Vec<usize> = (0..n).collect();
    perm.sort_by_key(|&v| {
        let degree = hg.vertex_hyperedge_offsets[v + 1] - hg.vertex_hyperedge_offsets[v];
        (degree, hg.vertex_weights[v])
    });
    // Shuffle within each equal-degree run: the degree order itself is what
    // this wants, but leaving ties in vertex-index order makes every level of
    // every restart match the same pairs first.
    {
        let mut i = 0;
        while i < n {
            let mut j = i + 1;
            let degree =
                hg.vertex_hyperedge_offsets[perm[i] + 1] - hg.vertex_hyperedge_offsets[perm[i]];
            while j < n
                && (hg.vertex_hyperedge_offsets[perm[j] + 1] - hg.vertex_hyperedge_offsets[perm[j]])
                    == degree
            {
                j += 1;
            }
            for k in (i + 1..j).rev() {
                let l = i + (rng.next_u64() as usize) % (k - i + 1);
                perm.swap(k, l);
            }
            i = j;
        }
    }

    let mut match_of = vec![None; n];
    let mut coarse_id: Vec<u32> = vec![0; n];
    let mut num_coarse: u32 = 0;

    for &v in &perm {
        if match_of[v].is_some() {
            continue;
        }

        // Candidates are ranked by the total weight of the hyperedges they
        // share with `v`. This makes an explicitly weighted hyperedge, including
        // canonicalized repeats whose weights were added, influence coarsening
        // by the same amount that it influences the cut objective.
        let mut connectivity: FxHashMap<u32, u32> = FxHashMap::default();

        for &hei in hg.vertex_hyperedges(v) {
            let weight = hg.hyperedge_weights[hei as usize];
            for &u in hg.charged_hyperedge_pins(hei as usize) {
                let u = u as usize;
                if u != v && match_of[u].is_none() {
                    let shared_weight = connectivity.entry(u as u32).or_insert(0);
                    *shared_weight = shared_weight
                        .checked_add(weight)
                        .expect("validated total hyperedge weight fits in u32");
                }
            }
        }

        // An explicit lowest-index tie-break makes selection independent of
        // hash-table iteration order.
        let mut best_neighbor = None;
        let mut best_conn: u32 = 0;
        let mut best_same_part = false;
        for (&nb, &conn) in &connectivity {
            let same_part = part.is_some_and(|p| p[nb as usize] == p[v]);
            if same_part && !best_same_part {
                best_conn = conn;
                best_neighbor = Some(nb);
                best_same_part = true;
            } else if same_part == best_same_part
                && (conn > best_conn
                    || (conn == best_conn && best_neighbor.is_none_or(|best| nb < best)))
            {
                best_conn = conn;
                best_neighbor = Some(nb);
            }
        }

        if let Some(neighbor) = best_neighbor {
            match_of[v] = Some(neighbor);
            match_of[neighbor as usize] = Some(v as u32);
            coarse_id[v] = num_coarse;
            coarse_id[neighbor as usize] = num_coarse;
            num_coarse += 1;
        } else {
            // Every neighbour already matched: the vertex crosses the level
            // alone, which is why a level can shrink by less than half and why
            // the floor below is needed at all.
            match_of[v] = Some(v as u32);
            coarse_id[v] = num_coarse;
            num_coarse += 1;
        }
    }

    let nc = num_coarse as usize;
    if nc >= n * 9 / 10 {
        return None; // tuned 10% floor: stop once a level barely shrinks
    }

    let mut coarse_vertex_weights = vec![0u32; nc];
    for v in 0..n {
        coarse_vertex_weights[coarse_id[v] as usize] += hg.vertex_weights[v];
    }

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

    // Sorting by pin set brings identical coarse hyperedges together so the
    // scan below can merge them by summing weights: two hyperedges that this
    // contraction made indistinguishable are one hyperedge of their combined
    // weight from here up.
    weighted_hyperedges.sort_by(|a, b| a.0.cmp(&b.0));
    let mut coarse_hyperedges: Vec<Vec<u32>> = Vec::new();
    let mut coarse_hyperedge_weights: Vec<u32> = Vec::new();
    for (pins, weight) in weighted_hyperedges {
        if !coarse_hyperedges.is_empty() && *coarse_hyperedges.last().unwrap() == pins {
            let combined = coarse_hyperedge_weights.last_mut().unwrap();
            *combined = combined
                .checked_add(weight)
                .expect("coarsening preserves the validated total hyperedge weight");
        } else {
            coarse_hyperedges.push(pins);
            coarse_hyperedge_weights.push(weight);
        }
    }

    let mut coarse_hg =
        Hypergraph::from_hyperedges(nc, &coarse_hyperedges, Some(&coarse_hyperedge_weights));
    coarse_hg.vertex_weights = coarse_vertex_weights;

    Some(CoarseningLevel {
        hg: coarse_hg,
        mapping: coarse_id,
    })
}
