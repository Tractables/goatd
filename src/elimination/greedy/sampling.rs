//! The two tie-set-sampling cores.
//!
//! These stay outside `greedy.rs`'s skeleton on purpose. They do not pop from
//! a heap: they draw a vertex at random from the minimum-priority bucket of a
//! [`BucketMap`] — or, with a `band`, from every bucket within `band` of the
//! minimum — which means no lazy deletion, no stale entries to skip, and a
//! priority structure that has to be kept exact rather than corrected on pop.
//! `eliminate_sampled_min_fill` also has no clique-residual fast drain — at
//! fill 0 every remaining vertex ties, and draining them in index order would
//! change the sampled order. Folding them into the skeleton would mean more
//! hooks that only they use, and the sampling loop would be harder to read
//! for it, not easier.

use super::*;
use crate::deadline::expired;
use crate::rng::{SEED_OFFSET, Xorshift64};

/// How a sampled core draws: the weights that bias the tie set, how far above
/// the minimum score the tie set reaches, and which stream the draws come
/// from.
#[derive(Clone, Copy)]
pub(crate) struct SampleDraw<'a> {
    /// One weight per graph vertex; a smaller weight is drawn more often.
    pub(crate) weights: &'a [u32],
    /// The band above the minimum score, in the score's own units. 0 draws
    /// from the vertices tied at the minimum, as the samplers always have.
    pub(crate) band: u64,
    /// Selects the RNG stream the tie-set draws come from.
    pub(crate) seed: u64,
}

#[derive(Clone, Copy)]
enum FillPriority {
    Fill,
    FillDegree(i8),
}

impl FillPriority {
    fn key(self, fill: u64, degree: u64, vertex_count: u64) -> u64 {
        match self {
            Self::Fill => fill,
            Self::FillDegree(degree_coefficient) if degree_coefficient >= 0 => {
                fill.saturating_add(degree.saturating_mul(degree_coefficient as u64))
            }
            Self::FillDegree(degree_coefficient) => {
                debug_assert!(degree <= vertex_count);
                fill.saturating_add(
                    (vertex_count - degree)
                        .saturating_mul(u64::from(degree_coefficient.unsigned_abs())),
                )
            }
        }
    }

    fn tracks_fill_separately(self) -> bool {
        !matches!(self, Self::Fill)
    }
}

/// Move every neighbour of the eliminated vertex to the bucket of its updated
/// fill, in neighbour order. This is the eager half of
/// `eliminate_sampled_min_fill`'s bucket maintenance — the sampler cannot
/// tolerate a stale key, since a stale bucket biases which vertex gets
/// sampled, not just which entry pops first.
fn update_neighbours(
    affected: &mut FillAffected,
    graph: &EliminationGraph,
    nbrs: &[u32],
    buckets: &mut BucketMap<'_>,
    fills: &mut Option<&mut [u64]>,
    priority: FillPriority,
) {
    for &u in nbrs {
        if graph.active[u as usize] {
            let old_fill = match fills.as_deref() {
                Some(fills) => fills[u as usize],
                None => buckets
                    .key_of(u)
                    .expect("an active vertex has a fill bucket"),
            };
            let new_fill = affected.neighbour_fill(graph, u, old_fill);
            if let Some(fills) = fills.as_deref_mut() {
                fills[u as usize] = new_fill;
            }
            buckets.update(
                u,
                priority.key(new_fill, graph.degree(u) as u64, graph.len() as u64),
            );
        }
    }
}

/// File one vertex in its starting bucket, and record its fill where the score
/// keeps it separately. Shared by the two ways the seeding loop below reaches a
/// vertex, so both file it identically.
#[inline]
fn seed_bucket(
    graph: &EliminationGraph,
    v: u32,
    initial_fill: Option<&[u64]>,
    fill_scratch: &mut FillScratch,
    fills: &mut Option<&mut [u64]>,
    buckets: &mut BucketMap<'_>,
    priority: FillPriority,
) {
    let f = match initial_fill {
        Some(f) => f[v as usize],
        None => fill_scratch.fill_count_of(graph, v),
    };
    if let Some(fills) = fills.as_deref_mut() {
        fills[v as usize] = f;
    }
    buckets.insert(
        v,
        priority.key(f, graph.degree(v) as u64, graph.len() as u64),
    );
}

/// htd-style min-fill elimination: priority = fill only (no secondary degree
/// or salt key), ties broken by random sampling from the full min-fill tie
/// set. A smaller `weights[v]` makes `v` more likely to be drawn.
///
/// `band` widens the tie set to every vertex whose fill is at most `band` above
/// the smallest; 0 is the exact minimum.
pub(crate) fn eliminate_sampled_min_fill(
    graph: &mut EliminationGraph,
    draw: SampleDraw<'_>,
    sink: ElimSink<'_>,
    stop: ElimStop,
    initial_fill: Option<&[u64]>,
    active: Option<&[u32]>,
    scratch: &mut SampleScratch,
) -> ElimExit {
    eliminate_sampled_fill_based(
        graph,
        draw,
        sink,
        stop,
        initial_fill,
        active,
        FillPriority::Fill,
        scratch,
    )
}

/// Fill-plus-coefficient-times-degree elimination with weighted sampling from
/// the complete minimum-score tie set, or from a `band` above it.
pub(crate) fn eliminate_sampled_fill_degree(
    graph: &mut EliminationGraph,
    draw: SampleDraw<'_>,
    sink: ElimSink<'_>,
    stop: ElimStop,
    initial_fill: Option<&[u64]>,
    active: Option<&[u32]>,
    degree_coefficient: i8,
    scratch: &mut SampleScratch,
) -> ElimExit {
    eliminate_sampled_fill_based(
        graph,
        draw,
        sink,
        stop,
        initial_fill,
        active,
        FillPriority::FillDegree(degree_coefficient),
        scratch,
    )
}

fn eliminate_sampled_fill_based(
    graph: &mut EliminationGraph,
    draw: SampleDraw<'_>,
    mut sink: ElimSink<'_>,
    stop: ElimStop,
    initial_fill: Option<&[u64]>,
    active: Option<&[u32]>,
    priority: FillPriority,
    scratch: &mut SampleScratch,
) -> ElimExit {
    let SampleDraw {
        weights,
        band,
        seed,
    } = draw;
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

    let SampleScratch {
        fill: fill_scratch,
        affected,
        buckets: bucket_storage,
        live_nbrs,
        fills: fill_store,
        ..
    } = scratch;
    fill_scratch.size_for(n);
    affected.size_for(n);
    // Plain min-fill already stores the current fill as the bucket key. Only
    // composite scores need a second array to recover the fill component.
    //
    // The entries are written for every active vertex by the seeding loop below
    // and read only at active vertices, so the store is grown to the graph and
    // left holding whatever the run before it wrote.
    let mut fills: Option<&mut [u64]> = if priority.tracks_fill_separately() {
        if fill_store.len() < n {
            fill_store.resize(n, 0);
        }
        Some(&mut fill_store[..n])
    } else {
        None
    };
    let mut buckets = BucketMap::with_weights(bucket_storage, weights, uniform_mass);
    match active {
        // The caller's list of active vertices in index order, which is the
        // order the scan below reaches them in.
        Some(active) => {
            for &v in active {
                debug_assert!(graph.active[v as usize]);
                seed_bucket(
                    graph,
                    v,
                    initial_fill,
                    fill_scratch,
                    &mut fills,
                    &mut buckets,
                    priority,
                );
            }
        }
        None => {
            for v in 0..n as u32 {
                if graph.active[v as usize] {
                    seed_bucket(
                        graph,
                        v,
                        initial_fill,
                        fill_scratch,
                        &mut fills,
                        &mut buckets,
                        priority,
                    );
                }
            }
        }
    }

    // `+ SEED_OFFSET` keeps a seed of 0 off xorshift64's zero fixed point, and
    // is part of the tie-break stream this sampler has always drawn.
    let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
    let mut pacer = DeadlinePacer::new();

    while buckets.minimum().is_some() {
        if pacer.due() {
            if expired(hard_deadline) {
                return ElimExit::DeadlineReached(Cutoff::Hard);
            }
            if graph.should_promote_bitset() {
                graph.promote_bitset();
            }
        }

        let v = buckets
            .sample_min_band(band, &mut rng)
            .expect("a live minimum has a tie set");
        // The drawn vertex's own fill, not the band's smallest: the simplicial
        // path below is only correct for a vertex that adds no fill edge, and a
        // band wider than 0 can draw one that does.
        let sampled_fill = fills.as_deref().map_or_else(
            || {
                buckets
                    .key_of(v)
                    .expect("a sampled vertex has a fill bucket")
            },
            |fills| fills[v as usize],
        );

        buckets.remove_vertex(v);

        let bag = take_bag(graph, v, live_nbrs);
        let bag_len = bag.len();
        // Recorded before the elimination below removes `v`, because the score
        // repair that follows reads the deadline and returns from the middle of
        // it. A return after the removal and before the record left `v` out of
        // the bags entirely, and the engine's residual bag does not catch it:
        // that bag holds what is still in the graph, which `v` no longer is.
        sink.record(v, bag);

        if sampled_fill == 0 {
            // N(v) is a clique: no fill edge is added, and each neighbour
            // loses the missing pairs it had with v alone.
            affected.prepare(graph, v, live_nbrs, false, None);
            graph.remove_without_fill_nbrs(v, live_nbrs);
            update_neighbours(
                affected,
                graph,
                live_nbrs,
                &mut buckets,
                &mut fills,
                priority,
            );
        } else {
            // `v` is recorded above, so it leaves the graph even on the
            // deadline exit below: the residual bag that exit builds holds
            // what is still in the graph, and `v` must not appear twice.
            if !affected.prepare(graph, v, live_nbrs, true, hard_deadline) {
                graph.eliminate_with_nbrs(v, live_nbrs);
                return ElimExit::DeadlineReached(Cutoff::Hard);
            }
            graph.eliminate_prepared(v, live_nbrs, &affected.fill_edges());
            // Applying one delta is a bucket move, so this loop reads the
            // deadline on the pacer's stride.
            let mut delta_pacer = DeadlinePacer::new();
            while let Some((u, delta)) = affected.pop_delta(graph) {
                if delta_pacer.due() && expired(hard_deadline) {
                    // The neighbour terms this run's last prepare left are
                    // never read now, and the scratch outlives the run, so
                    // they go back to zero here rather than at the next use.
                    affected.clear();
                    affected.clear_neighbours(graph, live_nbrs);
                    return ElimExit::DeadlineReached(Cutoff::Hard);
                }
                let old_fill = fills.as_deref().map_or_else(
                    || {
                        buckets
                            .key_of(u)
                            .expect("an active vertex has a fill bucket")
                    },
                    |fills| fills[u as usize],
                );
                debug_assert!(delta <= old_fill);
                let new_fill = old_fill.saturating_sub(delta);
                if let Some(fills) = fills.as_deref_mut() {
                    fills[u as usize] = new_fill;
                }
                buckets.update(
                    u,
                    priority.key(new_fill, graph.degree(u) as u64, graph.len() as u64),
                );
            }
            update_neighbours(
                affected,
                graph,
                live_nbrs,
                &mut buckets,
                &mut fills,
                priority,
            );
        }
        if exceeds_width_bound(bag_len, width_bound) {
            return ElimExit::WidthLimitExceeded;
        }
    }
    ElimExit::Complete
}

/// htd-style min-degree elimination: priority = degree only, ties broken by
/// random sampling from the full min-degree tie set. `weights` biases the
/// sample and `band` widens the tie set by degree, both as in
/// `eliminate_sampled_min_fill`.
pub(crate) fn eliminate_sampled_min_degree(
    graph: &mut EliminationGraph,
    draw: SampleDraw<'_>,
    mut sink: ElimSink<'_>,
    stop: ElimStop,
    active: Option<&[u32]>,
    scratch: &mut SampleScratch,
) -> ElimExit {
    let SampleDraw {
        weights,
        band,
        seed,
    } = draw;
    // As in `eliminate_sampled_min_fill`: no cheap mode, so no soft deadline.
    let ElimStop {
        hard_deadline,
        width_bound,
        ..
    } = stop;
    let n = graph.len();
    assert_eq!(weights.len(), n);
    let uniform_mass = uniform_sampling_mass(weights);

    let SampleScratch {
        buckets: bucket_storage,
        live_nbrs: nbrs_buf,
        degree_stale,
        ..
    } = scratch;
    let mut buckets = BucketMap::with_weights(bucket_storage, weights, uniform_mass);
    match active {
        // The caller's list of active vertices in index order, which is the
        // order the scan below reaches them in.
        Some(active) => {
            for &v in active {
                debug_assert!(graph.active[v as usize]);
                buckets.insert(v, graph.degree(v) as u64);
            }
        }
        None => {
            for v in 0..n as u32 {
                if graph.active[v as usize] {
                    buckets.insert(v, graph.degree(v) as u64);
                }
            }
        }
    }

    // `+ SEED_OFFSET` keeps a seed of 0 off xorshift64's zero fixed point, and
    // is part of the tie-break stream this sampler has always drawn.
    let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
    let mut pacer = DeadlinePacer::new();
    let mut clique_residual = false;
    // Lazy degree tracking — defer bucket update to sample time. The flags are
    // grown to the graph and left as the run before them wrote them: the
    // seeding above files every degree exactly, and the elimination marks every
    // vertex whose degree it changes, so a flag left set costs one comparison
    // that finds the filed degree already right and changes nothing.
    if degree_stale.len() < n {
        degree_stale.resize(n, false);
    }
    let degree_stale = &mut degree_stale[..n];

    while buckets.minimum().is_some() {
        if pacer.due() && expired(hard_deadline) {
            return ElimExit::DeadlineReached(Cutoff::Hard);
        }

        let v = buckets
            .sample_min_band(band, &mut rng)
            .expect("a live minimum has a tie set");
        let vi = v as usize;

        if degree_stale[vi] {
            let live_degree = graph.degree(v) as u64;
            degree_stale[vi] = false;
            let filed_degree = buckets
                .key_of(v)
                .expect("a sampled vertex has a degree bucket");
            if live_degree != filed_degree {
                buckets.update(v, live_degree);
                continue;
            }
        }

        buckets.remove_vertex(v);

        let bag = take_bag(graph, v, nbrs_buf);
        let bag_len = bag.len();

        if !clique_residual && graph.is_residual_clique() {
            clique_residual = true;
        }
        if clique_residual {
            graph.remove_without_fill_nbrs(v, nbrs_buf);
        } else {
            graph.eliminate_with_nbrs(v, nbrs_buf);
        }
        sink.record(v, bag);

        if exceeds_width_bound(bag_len, width_bound) {
            return ElimExit::WidthLimitExceeded;
        }

        for &u in nbrs_buf.iter() {
            degree_stale[u as usize] = true;
        }
    }
    ElimExit::Complete
}
