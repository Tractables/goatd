//! The two tie-set-sampling cores.
//!
//! These stay outside `greedy.rs`'s skeleton on purpose. They do not pop from
//! a heap: they read the whole minimum-priority bucket out of a [`BucketMap`]
//! and draw a vertex from it at random, which means no lazy deletion, no stale
//! entries to skip, and a priority structure that has to be kept exact rather
//! than corrected on pop. `eliminate_sampled_min_fill` also has no
//! clique-residual fast drain — at fill 0 every remaining vertex ties, and
//! draining them in index order would change the sampled order — and updates
//! its buckets *before* the vertex is
//! removed on the bitset path, where the skeleton updates after. Folding them
//! in would mean two more hooks that only they use, and the sampling loop
//! would be harder to read for it, not easier.

use super::*;
use crate::deadline::expired;
use crate::rng::{SEED_OFFSET, Xorshift64};

#[derive(Clone, Copy)]
enum FillPriority {
    Fill,
    DegreePlusFill,
    SparsestSubgraph,
    FillMinusDoubleDegree,
}

impl FillPriority {
    fn key(self, fill: u64, degree: u64, vertex_count: u64) -> u64 {
        match self {
            Self::Fill => fill,
            Self::DegreePlusFill => fill.saturating_add(degree),
            Self::SparsestSubgraph => {
                debug_assert!(degree <= vertex_count);
                fill + vertex_count - degree
            }
            Self::FillMinusDoubleDegree => {
                debug_assert!(degree <= vertex_count);
                fill.saturating_add((vertex_count - degree).saturating_mul(2))
            }
        }
    }

    fn tracks_fill_separately(self) -> bool {
        !matches!(self, Self::Fill)
    }
}

/// Re-measure every still-active neighbour's fill and move it to the matching
/// bucket. This is the eager half of `eliminate_sampled_min_fill`'s bucket
/// maintenance — the sampler cannot tolerate a stale key, since a stale bucket
/// biases which vertex gets sampled, not just which entry pops first.
fn rescore_neighbours(
    scratch: &mut FillScratch,
    graph: &EliminationGraph,
    nbrs: &[u32],
    buckets: &mut BucketMap<'_>,
    fills: &mut Option<Vec<u64>>,
    priority: FillPriority,
) {
    for &u in nbrs {
        if graph.active[u as usize] {
            let new_fill = scratch.fill_count_of(graph, u);
            if let Some(fills) = fills {
                fills[u as usize] = new_fill;
            }
            buckets.update(
                u,
                priority.key(new_fill, graph.degree(u) as u64, graph.len() as u64),
            );
        }
    }
}

/// htd-style min-fill elimination: priority = fill only (no secondary degree
/// or salt key), ties broken by random sampling from the full min-fill tie
/// set. A smaller `weights[v]` makes `v` more likely to be drawn.
pub(crate) fn eliminate_sampled_min_fill(
    graph: &mut EliminationGraph,
    weights: &[u32],
    seed: u64,
    sink: ElimSink<'_>,
    stop: ElimStop,
    initial_fill: Option<&[u64]>,
) -> ElimExit {
    eliminate_sampled_fill_based(
        graph,
        weights,
        seed,
        sink,
        stop,
        initial_fill,
        FillPriority::Fill,
    )
}

/// Degree-plus-fill elimination with weighted sampling from the complete
/// minimum-score tie set.
pub(crate) fn eliminate_sampled_degree_plus_fill(
    graph: &mut EliminationGraph,
    weights: &[u32],
    seed: u64,
    sink: ElimSink<'_>,
    stop: ElimStop,
    initial_fill: Option<&[u64]>,
) -> ElimExit {
    eliminate_sampled_fill_based(
        graph,
        weights,
        seed,
        sink,
        stop,
        initial_fill,
        FillPriority::DegreePlusFill,
    )
}

/// Fill-minus-degree elimination with weighted sampling from the complete
/// minimum-score tie set.
pub(crate) fn eliminate_sampled_sparsest_subgraph(
    graph: &mut EliminationGraph,
    weights: &[u32],
    seed: u64,
    sink: ElimSink<'_>,
    stop: ElimStop,
    initial_fill: Option<&[u64]>,
) -> ElimExit {
    eliminate_sampled_fill_based(
        graph,
        weights,
        seed,
        sink,
        stop,
        initial_fill,
        FillPriority::SparsestSubgraph,
    )
}

/// Fill-minus-double-degree elimination with weighted sampling from the
/// complete minimum-score tie set.
pub(crate) fn eliminate_sampled_fill_minus_double_degree(
    graph: &mut EliminationGraph,
    weights: &[u32],
    seed: u64,
    sink: ElimSink<'_>,
    stop: ElimStop,
    initial_fill: Option<&[u64]>,
) -> ElimExit {
    eliminate_sampled_fill_based(
        graph,
        weights,
        seed,
        sink,
        stop,
        initial_fill,
        FillPriority::FillMinusDoubleDegree,
    )
}

fn eliminate_sampled_fill_based(
    graph: &mut EliminationGraph,
    weights: &[u32],
    seed: u64,
    mut sink: ElimSink<'_>,
    stop: ElimStop,
    initial_fill: Option<&[u64]>,
    priority: FillPriority,
) -> ElimExit {
    // No cheap mode here to degrade into, so the soft deadline is not this
    // core's to read.
    let ElimStop {
        hard_deadline,
        width_bound,
        ..
    } = stop;
    let n = graph.len();
    assert_eq!(weights.len(), n);
    let uniform_mass = uniform_sampling_mass(weights);

    if graph.should_promote_bitset() {
        graph.promote_bitset();
    }

    let mut scratch = FillScratch::new(n);
    let mut affected = FillAffected::new(n);
    let mut fill_edges = Vec::new();
    let mut live_nbrs = Vec::new();
    // Plain min-fill already stores the current fill as the bucket key. Only
    // composite scores need a second array to recover the fill component.
    let mut fills = priority.tracks_fill_separately().then(|| vec![0; n]);
    let mut buckets = BucketMap::with_weights(weights);
    for v in 0..n {
        if graph.active[v] {
            let f = match initial_fill {
                Some(f) => f[v],
                None => scratch.fill_count_of(graph, v as u32),
            };
            if let Some(fills) = &mut fills {
                fills[v] = f;
            }
            buckets.insert(
                v as u32,
                priority.key(f, graph.degree(v as u32) as u64, n as u64),
            );
        }
    }

    // `+ SEED_OFFSET` keeps a seed of 0 off xorshift64's zero fixed point, and
    // is part of the tie-break stream this sampler has always drawn.
    let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
    let mut check_counter = 0u32;

    while let Some((minimum_priority, tie_set, total_mass)) = buckets.min_bucket() {
        check_counter += 1;
        if check_counter >= DEADLINE_CHECK_STRIDE {
            check_counter = 0;
            if expired(hard_deadline) {
                return ElimExit::DeadlineReached;
            }
            if graph.should_promote_bitset() {
                graph.promote_bitset();
            }
        }

        let v = sample_tie_set(tie_set, weights, &mut rng, uniform_mass, total_mass);
        let min_fill = fills
            .as_ref()
            .map_or(minimum_priority, |fills| fills[v as usize]);

        buckets.remove_vertex(v);

        let bag = take_bag(graph, v, &mut live_nbrs);

        if min_fill == 0 {
            // Bitset mode: exact Δfill update in O(w) before removing v.
            // When v is simplicial, N(v) is a clique so no fill edges are added.
            // Δfill(u) = -|N(u) \ N(v) \ {v}| = -(popcount(bs[u] & ~bs[v]) - 1).
            if graph.bitset_words > 0 {
                let w = graph.bitset_words;
                let vb = v as usize * w;
                for &u in &live_nbrs {
                    let ui = u as usize;
                    let ub = ui * w;
                    let mut o_count = 0u64;
                    for j in 0..w {
                        o_count +=
                            (graph.bitset[ub + j] & !graph.bitset[vb + j]).count_ones() as u64;
                    }
                    o_count = o_count.saturating_sub(1); // exclude v's own bit
                    if let Some(fills) = &mut fills {
                        fills[ui] = fills[ui].saturating_sub(o_count);
                    } else {
                        let old_fill = buckets
                            .key_of(u)
                            .expect("an active vertex has a fill bucket");
                        buckets.update(u, old_fill.saturating_sub(o_count));
                    }
                }
                graph.remove_without_fill_nbrs(v, &live_nbrs);
                if let Some(fills) = &fills {
                    for &u in &live_nbrs {
                        buckets.update(
                            u,
                            priority.key(
                                fills[u as usize],
                                graph.degree(u) as u64,
                                graph.len() as u64,
                            ),
                        );
                    }
                }
            } else {
                graph.remove_without_fill_nbrs(v, &live_nbrs);
                rescore_neighbours(
                    &mut scratch,
                    graph,
                    &live_nbrs,
                    &mut buckets,
                    &mut fills,
                    priority,
                );
            }
        } else {
            graph.eliminate_with_nbrs_record_fill(v, &live_nbrs, &mut fill_edges);
            if !affected.collect_deltas(graph, &live_nbrs, &fill_edges, hard_deadline) {
                return ElimExit::DeadlineReached;
            }
            while let Some((u, delta)) = affected.pop_delta() {
                if expired(hard_deadline) {
                    affected.clear();
                    return ElimExit::DeadlineReached;
                }
                let old_fill = fills.as_ref().map_or_else(
                    || {
                        buckets
                            .key_of(u)
                            .expect("an active vertex has a fill bucket")
                    },
                    |fills| fills[u as usize],
                );
                debug_assert!(delta <= old_fill);
                let new_fill = old_fill.saturating_sub(delta);
                if let Some(fills) = &mut fills {
                    fills[u as usize] = new_fill;
                }
                buckets.update(
                    u,
                    priority.key(new_fill, graph.degree(u) as u64, graph.len() as u64),
                );
            }
            for &u in &live_nbrs {
                if expired(hard_deadline) {
                    return ElimExit::DeadlineReached;
                }
                if graph.active[u as usize] {
                    let new_fill = scratch.fill_count_of(graph, u);
                    if let Some(fills) = &mut fills {
                        fills[u as usize] = new_fill;
                    }
                    buckets.update(
                        u,
                        priority.key(new_fill, graph.degree(u) as u64, graph.len() as u64),
                    );
                }
            }
        }
        let bag_len = bag.len();
        sink.record(v, bag);

        if exceeds_width_bound(bag_len, width_bound) {
            return ElimExit::WidthLimitExceeded;
        }
    }
    ElimExit::Complete
}

/// htd-style min-degree elimination: priority = degree only, ties broken by
/// random sampling from the full min-degree tie set. `weights` biases the
/// sample (see `eliminate_sampled_min_fill`).
pub(crate) fn eliminate_sampled_min_degree(
    graph: &mut EliminationGraph,
    weights: &[u32],
    seed: u64,
    mut sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    // As in `eliminate_sampled_min_fill`: no cheap mode, so no soft deadline.
    let ElimStop {
        hard_deadline,
        width_bound,
        ..
    } = stop;
    let n = graph.len();
    assert_eq!(weights.len(), n);
    let uniform_mass = uniform_sampling_mass(weights);

    let mut buckets = BucketMap::with_weights(weights);
    for v in 0..n {
        if graph.active[v] {
            buckets.insert(v as u32, graph.degree(v as u32) as u64);
        }
    }

    // `+ SEED_OFFSET` keeps a seed of 0 off xorshift64's zero fixed point, and
    // is part of the tie-break stream this sampler has always drawn.
    let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
    let mut nbrs_buf = Vec::new();
    let mut check_counter = 0u32;
    let mut clique_residual = false;
    // Lazy degree tracking — defer bucket update to sample time.
    let mut degree_stale: Vec<bool> = vec![false; n];

    while let Some((min_deg, tie_set, total_mass)) = buckets.min_bucket() {
        check_counter += 1;
        if check_counter >= DEADLINE_CHECK_STRIDE {
            check_counter = 0;
            if expired(hard_deadline) {
                return ElimExit::DeadlineReached;
            }
        }

        let v = sample_tie_set(tie_set, weights, &mut rng, uniform_mass, total_mass);
        let vi = v as usize;

        if degree_stale[vi] {
            let live_degree = graph.degree(v) as u64;
            degree_stale[vi] = false;
            if live_degree != min_deg {
                buckets.update(v, live_degree);
                continue;
            }
        }

        buckets.remove_vertex(v);

        let bag = take_bag(graph, v, &mut nbrs_buf);
        let bag_len = bag.len();

        if !clique_residual && graph.is_residual_clique() {
            clique_residual = true;
        }
        if clique_residual {
            graph.remove_without_fill_nbrs(v, &nbrs_buf);
        } else {
            graph.eliminate_with_nbrs(v, &nbrs_buf);
        }
        sink.record(v, bag);

        if exceeds_width_bound(bag_len, width_bound) {
            return ElimExit::WidthLimitExceeded;
        }

        for &u in &nbrs_buf {
            degree_stale[u as usize] = true;
        }
    }
    ElimExit::Complete
}
