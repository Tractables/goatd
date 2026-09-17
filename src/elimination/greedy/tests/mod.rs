//! Beside the module: these drive the private elimination sink directly.

mod min_fill;
mod relative_fill;

/// The tie set a draw within `band` of the minimum sees, materialised: the
/// band's buckets concatenated ascending by key, and their combined mass.
/// The sampler itself walks the same buckets without building this.
fn tie_set(buckets: &mut super::BucketMap<'_>, band: u64) -> Option<(Vec<u32>, u64)> {
    let minimum = buckets.minimum()?;
    let mut vertices = Vec::new();
    let mut mass = 0;
    for bucket in buckets.band_buckets(minimum, band) {
        vertices.extend_from_slice(&bucket.vertices);
        mass += buckets.bucket_mass(bucket);
    }
    Some((vertices, mass))
}

#[test]
fn sampling_mass_prefers_smaller_public_weights() {
    assert_eq!(super::sampling_mass(0), u64::from(u32::MAX) + 1);
    assert_eq!(super::sampling_mass(u32::MAX), 1);
    assert!(super::sampling_mass(7) > super::sampling_mass(8));
}

#[test]
fn uniform_sampling_repeats_the_generic_weighted_choices() {
    let weights = vec![7; 64];
    let uniform_mass = super::uniform_sampling_mass(&weights).expect("equal weights");
    let mut fast = crate::rng::Xorshift64::from_state(17);
    let mut generic = fast;

    for len in 2..=weights.len() {
        let mut fast_storage = super::BucketStorage::new();
        let mut generic_storage = super::BucketStorage::new();
        let mut fast_buckets =
            super::BucketMap::with_weights(&mut fast_storage, &weights, Some(uniform_mass));
        let mut generic_buckets =
            super::BucketMap::with_weights(&mut generic_storage, &weights, None);
        for v in 0..len as u32 {
            fast_buckets.insert(v, 3);
            generic_buckets.insert(v, 3);
        }
        assert_eq!(
            fast_buckets.sample_min_band(0, &mut fast),
            generic_buckets.sample_min_band(0, &mut generic),
            "tie-set length {len}",
        );
    }
}

#[test]
fn unequal_sampling_weights_do_not_enable_the_uniform_path() {
    assert_eq!(super::uniform_sampling_mass(&[1, 1, 2, 1]), None);
}

#[test]
fn priority_buckets_track_their_weighted_sampling_mass() {
    let weights = [0, u32::MAX, 17, 42];
    let mut storage = super::BucketStorage::new();
    let mut buckets = super::BucketMap::with_weights(&mut storage, &weights, None);
    buckets.insert(0, 3);
    buckets.insert(1, 3);
    buckets.insert(2, 3);

    let (vertices, total_mass) = tie_set(&mut buckets, 0).unwrap();
    assert_eq!(vertices, [0, 1, 2]);
    assert_eq!(
        total_mass,
        super::sampling_mass(weights[0])
            + super::sampling_mass(weights[1])
            + super::sampling_mass(weights[2])
    );

    buckets.update(1, 7);
    buckets.remove_vertex(0);
    assert_eq!(buckets.minimum(), Some(3));
    let (vertices, total_mass) = tie_set(&mut buckets, 0).unwrap();
    assert_eq!(vertices, [2]);
    assert_eq!(total_mass, super::sampling_mass(weights[2]));
}

#[test]
fn uniform_priority_buckets_derive_mass_from_their_length() {
    let weights = [7, 7];
    let mass = super::sampling_mass(7);
    let mut storage = super::BucketStorage::new();
    let mut buckets = super::BucketMap::with_weights(&mut storage, &weights, Some(mass));
    buckets.insert(0, 3);
    buckets.insert(1, 3);

    assert_eq!(buckets.bucket(3).unwrap().sampling_mass, 0);
    assert_eq!(tie_set(&mut buckets, 0).unwrap().1, 2 * mass);
    buckets.remove_vertex(0);
    assert_eq!(tie_set(&mut buckets, 0).unwrap().1, mass);
}

#[test]
fn vacant_bucket_positions_do_not_reserve_a_priority_key() {
    let weights = [1];
    let mut storage = super::BucketStorage::new();
    let mut buckets =
        super::BucketMap::with_weights(&mut storage, &weights, Some(super::sampling_mass(1)));

    buckets.remove_vertex(0);
    buckets.insert(0, u64::MAX);
    assert_eq!(buckets.key_of(0), Some(u64::MAX));
    buckets.remove_vertex(0);
    assert_eq!(buckets.key_of(0), None);
}

#[test]
fn priority_buckets_recompute_an_emptied_minimum() {
    let weights = [1, 1];
    let mut storage = super::BucketStorage::new();
    let mut buckets =
        super::BucketMap::with_weights(&mut storage, &weights, Some(super::sampling_mass(1)));

    buckets.insert(0, 3);
    buckets.insert(1, 5);
    buckets.update(0, 7);
    assert_eq!(buckets.minimum(), Some(5));

    buckets.remove_vertex(1);
    assert_eq!(buckets.minimum(), Some(7));
}

/// A band wide enough to cross the dense boundary takes the slots and then
/// the overflow, ascending by key throughout, and a band that runs to the top
/// of the key space costs what the map holds rather than what it spans.
#[test]
fn a_band_across_the_dense_boundary_walks_its_buckets_in_key_order() {
    let weights = [1; 5];
    let mut storage = super::BucketStorage::new();
    let mut buckets =
        super::BucketMap::with_weights(&mut storage, &weights, Some(super::sampling_mass(1)));
    let dense = super::PriorityBuckets::dense_keys_for(weights.len()) as u64;

    buckets.insert(0, dense + 4);
    buckets.insert(1, 0);
    buckets.insert(2, dense);
    buckets.insert(3, dense - 1);
    buckets.insert(4, u64::MAX);

    let (vertices, mass) = tie_set(&mut buckets, u64::MAX).expect("a live minimum");
    assert_eq!(vertices, [1, 3, 2, 0, 4]);
    assert_eq!(mass, 5 * super::sampling_mass(1));

    // A band that stops inside the slots leaves the overflow where it is.
    let (vertices, _) = tie_set(&mut buckets, dense - 1).expect("a live minimum");
    assert_eq!(vertices, [1, 3]);
}

#[test]
fn priority_buckets_keep_their_slots_when_a_key_overflows() {
    let weights = [1, 1];
    let mut storage = super::BucketStorage::new();
    let mut buckets =
        super::BucketMap::with_weights(&mut storage, &weights, Some(super::sampling_mass(1)));

    buckets.insert(0, 3);
    buckets.insert(1, u64::MAX);
    assert!(buckets.buckets.overflow.contains_key(&u64::MAX));
    assert_eq!(buckets.minimum(), Some(3));

    buckets.remove_vertex(0);
    assert_eq!(buckets.minimum(), Some(u64::MAX));
    let (vertices, _) = tie_set(&mut buckets, 0).unwrap();
    assert_eq!(vertices, [1]);
}

/// The smallest key any of `vertices` is filed under, read back from the
/// positions rather than from the minimum the map tracks.
fn smallest_filed_key(buckets: &super::BucketMap<'_>, vertices: &[u32]) -> Option<u64> {
    vertices.iter().filter_map(|&v| buckets.key_of(v)).min()
}

#[test]
fn the_minimum_follows_the_smallest_key_across_the_dense_boundary() {
    let weights = [1; 6];
    let mut storage = super::BucketStorage::new();
    let mut buckets =
        super::BucketMap::with_weights(&mut storage, &weights, Some(super::sampling_mass(1)));
    let dense = super::PriorityBuckets::dense_keys_for(weights.len()) as u64;
    let all = [0, 1, 2, 3, 4, 5];

    // Two keys in the slots and three above them, one of those shared by two
    // vertices, filed in no particular order.
    buckets.insert(0, dense + 9);
    buckets.insert(1, dense + 2);
    buckets.insert(2, dense);
    buckets.insert(3, dense - 1);
    buckets.insert(4, 0);
    buckets.insert(5, dense + 2);
    assert_eq!(buckets.buckets.overflow.len(), 3, "keys above the slots");

    // Every removal but one takes the last vertex of the bucket the minimum
    // names: the slots run out at the third, and `dense + 2` keeps 1 at the
    // fourth.
    for v in [4, 3, 2, 5, 1, 0] {
        let expected = smallest_filed_key(&buckets, &all);
        assert_eq!(buckets.minimum(), expected, "before removing {v}");
        buckets.remove_vertex(v);
    }
    assert_eq!(buckets.minimum(), None);
}

#[test]
fn an_insert_below_the_minimum_takes_it_on_either_side_of_the_boundary() {
    let weights = [1; 4];
    let mut storage = super::BucketStorage::new();
    let mut buckets =
        super::BucketMap::with_weights(&mut storage, &weights, Some(super::sampling_mass(1)));
    let dense = super::PriorityBuckets::dense_keys_for(weights.len()) as u64;

    buckets.insert(0, dense + 8);
    assert_eq!(buckets.minimum(), Some(dense + 8));
    buckets.insert(1, dense + 3);
    assert_eq!(buckets.minimum(), Some(dense + 3));
    buckets.insert(2, dense + 5);
    assert_eq!(buckets.minimum(), Some(dense + 3), "an insert above it");

    // A move into the slots empties the bucket the minimum named.
    buckets.update(1, dense - 1);
    assert_eq!(buckets.minimum(), Some(dense - 1));
    assert_eq!(buckets.buckets.overflow.len(), 2);

    // Emptying that slot hands the minimum back to the smallest key above the
    // slots, which is not the one it left.
    buckets.remove_vertex(1);
    assert_eq!(buckets.minimum(), Some(dense + 5));
}

#[test]
fn reused_storage_drops_the_buckets_a_stopped_run_left_above_the_slots() {
    let weights = [1; 4];
    let mass = super::sampling_mass(1);
    let mut storage = super::BucketStorage::new();
    let dense = super::PriorityBuckets::dense_keys_for(weights.len()) as u64;
    {
        // A run that stops at its deadline leaves its vertices filed.
        let mut buckets = super::BucketMap::with_weights(&mut storage, &weights, Some(mass));
        buckets.insert(0, 1);
        buckets.insert(1, dense);
        buckets.insert(2, dense + 4);
        assert_eq!(buckets.buckets.overflow.len(), 2);
    }

    let mut buckets = super::BucketMap::with_weights(&mut storage, &weights, Some(mass));
    assert!(buckets.buckets.overflow.is_empty());
    assert_eq!(buckets.spare_vertices.len(), 2, "both buckets handed back");
    assert_eq!(buckets.minimum(), None);
    buckets.insert(0, dense + 1);
    assert_eq!(buckets.minimum(), Some(dense + 1));
}

#[test]
fn a_band_collects_the_buckets_above_the_minimum() {
    let weights = [1, 1, 1, 1];
    let mass = super::sampling_mass(1);
    let mut storage = super::BucketStorage::new();
    let mut buckets = super::BucketMap::with_weights(&mut storage, &weights, Some(mass));
    buckets.insert(0, 3);
    buckets.insert(1, 4);
    buckets.insert(2, 4);
    buckets.insert(3, 9);
    assert_eq!(tie_set(&mut buckets, 0).unwrap(), (vec![0], mass));
    let (vertices, total_mass) = tie_set(&mut buckets, 1).unwrap();
    assert_eq!(
        vertices,
        [0, 1, 2],
        "ascending by key, storage order inside"
    );
    assert_eq!(total_mass, 3 * mass);
    // Key 5 to 8 hold nothing, and the band stops before key 9.
    assert_eq!(tie_set(&mut buckets, 5).unwrap().0, [0, 1, 2]);
    assert_eq!(tie_set(&mut buckets, 6).unwrap().0, [0, 1, 2, 3]);
}

#[test]
fn a_band_collects_overflowing_buckets_too() {
    let weights = [1, 1];
    let mass = super::sampling_mass(1);
    let mut storage = super::BucketStorage::new();
    let mut buckets = super::BucketMap::with_weights(&mut storage, &weights, Some(mass));
    buckets.insert(0, u64::MAX - 1);
    buckets.insert(1, u64::MAX);
    assert_eq!(buckets.buckets.overflow.len(), 2);
    let (vertices, total_mass) = tie_set(&mut buckets, 1).unwrap();
    assert_eq!(vertices, [0, 1]);
    assert_eq!(total_mass, 2 * mass);
    // The top of the band saturates instead of wrapping past u64::MAX.
    assert_eq!(tie_set(&mut buckets, 4).unwrap().0, [0, 1]);
}

#[test]
fn bucket_positions_use_less_space_than_the_optional_tuple() {
    assert!(
        std::mem::size_of::<super::BucketPosition>() < std::mem::size_of::<Option<(u64, usize)>>()
    );
}

#[test]
fn empty_priority_buckets_reuse_their_vertex_storage() {
    let weights = [1];
    let mut storage = super::BucketStorage::new();
    let mut buckets =
        super::BucketMap::with_weights(&mut storage, &weights, Some(super::sampling_mass(1)));

    // A dense key hands its emptied bucket back on the free list.
    buckets.insert(0, 3);
    let capacity = buckets.bucket(3).unwrap().vertices.capacity();
    buckets.remove_vertex(0);
    assert_eq!(buckets.buckets.free.len(), 1);

    buckets.insert(0, 7);
    assert!(buckets.buckets.free.is_empty());
    assert_eq!(buckets.buckets.buckets.len(), 1);
    assert_eq!(buckets.bucket(7).unwrap().vertices.capacity(), capacity);

    // A key above the dense range keeps its storage in `spare_vertices`.
    buckets.update(0, u64::MAX);
    buckets.remove_vertex(0);
    assert_eq!(buckets.spare_vertices.len(), 1);
    buckets.insert(0, u64::MAX);
    assert!(buckets.spare_vertices.is_empty());
    assert_eq!(
        buckets.bucket(u64::MAX).unwrap().vertices.capacity(),
        capacity
    );
}

#[test]
fn affected_membership_uses_one_word_per_vertex_block() {
    let affected = super::FillAffected::new(130);
    assert_eq!(affected.inside.len(), 3);
}

#[test]
fn affected_membership_excludes_the_eliminated_neighbourhood() {
    // Eliminating 0 fills (1, 2). Their common neighbour 129 is outside N(0)
    // and loses a missing pair; 65 is inside and is updated as a neighbour.
    let mut graph = crate::elimination::graph::EliminationGraph::from_edges(
        130,
        &[
            (0, 1),
            (0, 2),
            (0, 65),
            (1, 65),
            (2, 65),
            (1, 129),
            (2, 129),
        ],
    );
    graph.promote_bitset();
    let mut affected = super::FillAffected::new(130);

    assert!(affected.prepare(&graph, 0, &[1, 2, 65], true, None));
    graph.eliminate_with_nbrs(0, &[1, 2, 65]);
    assert_eq!(affected.pop_delta(&graph), Some((129, 1)));
    assert_eq!(affected.pop_delta(&graph), None);
    // 1 keeps 129 and gains 2, which are adjacent: of its pairs (0, 129) and
    // (65, 129), the first goes with 0 and (2, 65) is an edge.
    assert_eq!(affected.neighbour_fill(&graph, 1, 2), 1);
    assert_eq!(affected.neighbour_fill(&graph, 2, 2), 1);
    // 65's one missing pair was (1, 2), now filled.
    assert_eq!(affected.neighbour_fill(&graph, 65, 1), 0);
}

/// Every score `FillAffected` maintains, checked against a fresh count over a
/// run of random eliminations, in both graph modes: the vertices an
/// elimination touched after each step, every active vertex every
/// `full_every` steps, and at most `steps` steps in all.
fn assert_updates_match_recounts(
    n: u32,
    edges: &[(u32, u32)],
    bitset: bool,
    seed: u64,
    steps: usize,
    full_every: usize,
) {
    let mut graph = crate::elimination::graph::EliminationGraph::from_edges(n, edges);
    if bitset && graph.bitset_words == 0 {
        graph.promote_bitset();
    }
    assert_eq!(graph.bitset_words > 0, bitset, "graph mode");
    let n = n as usize;
    let mut scratch = super::FillScratch::new(n);
    let mut affected = super::FillAffected::new(n);
    let mut fill: Vec<u64> = (0..n)
        .map(|v| scratch.fill_count_of(&graph, v as u32))
        .collect();
    let mut rng = crate::rng::Xorshift64::from_state(seed);
    let mut nbrs = Vec::new();
    let mut touched = Vec::new();
    for step in 1..=steps {
        if graph.num_active == 0 {
            break;
        }
        let live: Vec<u32> = (0..n as u32)
            .filter(|&v| graph.active[v as usize])
            .collect();
        let v = live[rng.below(live.len())];
        nbrs.clear();
        graph.collect_live_nbrs_into(v, &mut nbrs);
        if fill[v as usize] == 0 {
            assert!(affected.prepare(&graph, v, &nbrs, false, None));
            graph.remove_without_fill_nbrs(v, &nbrs);
        } else {
            assert!(affected.prepare(&graph, v, &nbrs, true, None));
            // The prepared elimination has to leave the graph exactly as
            // the one that finds the fill edges itself does.
            let mut twin = graph.clone();
            twin.eliminate_with_nbrs(v, &nbrs);
            graph.eliminate_prepared(v, &nbrs, &affected.fill_edges());
            assert!(
                graph.same_state_as(&twin),
                "prepared elimination of {v} left a different graph (bitset {bitset}, seed {seed})"
            );
        }
        touched.clear();
        while let Some((u, delta)) = affected.pop_delta(&graph) {
            assert!(!nbrs.contains(&u), "delta for a neighbour {u} of {v}");
            fill[u as usize] -= delta;
            touched.push(u);
        }
        for &u in &nbrs {
            fill[u as usize] = affected.neighbour_fill(&graph, u, fill[u as usize]);
            touched.push(u);
        }
        if step % full_every == 0 {
            touched.clear();
            touched.extend(live.iter().copied().filter(|&u| u != v));
        }
        for &u in &touched {
            assert_eq!(
                fill[u as usize],
                scratch.fill_count_of(&graph, u),
                "fill of {u} after eliminating {v} (bitset {bitset}, seed {seed})"
            );
        }
    }
}

fn random_edges(n: u32, m: usize, seed: u64) -> Vec<(u32, u32)> {
    let mut rng = crate::rng::Xorshift64::from_state(seed);
    let mut edges = Vec::with_capacity(m);
    while edges.len() < m {
        let u = rng.next_u32() % n;
        let w = rng.next_u32() % n;
        if u != w {
            edges.push((u.min(w), u.max(w)));
        }
    }
    edges
}

#[test]
fn neighbour_fill_updates_match_recounts() {
    for seed in 1..=6 {
        // Sparse enough to stay on rows, and dense enough to start on bits.
        let edges = random_edges(600, 1_500, seed);
        assert_updates_match_recounts(600, &edges, false, seed, 400, 1);
        let edges = random_edges(120, 600, seed);
        assert_updates_match_recounts(120, &edges, true, seed, 120, 1);
    }
}

/// A sparse graph with a hub whose row is long enough to be indexed, and the
/// same graph on a bitset re-indexed over the residual.
#[test]
fn neighbour_fill_updates_match_recounts_on_a_large_graph() {
    let mut edges = random_edges(17_000, 30_000, 9);
    edges.extend((1..400).map(|leaf| (0, leaf * 40)));
    assert_updates_match_recounts(17_000, &edges, false, 9, 300, 100);
    assert_updates_match_recounts(17_000, &edges, true, 9, 300, 100);
}

#[test]
fn the_initial_fill_pass_leaves_no_cache_when_it_is_abortable_and_the_clock_has_passed() {
    use std::time::{Duration, Instant};

    use crate::elimination::graph::EliminationGraph;

    let graph = EliminationGraph::from_edges(4, &[(0, 1), (1, 2), (2, 3), (0, 2)]);
    let past = Instant::now() - Duration::from_secs(1);
    let counted = |_vertex| 1;

    assert_eq!(
        super::initial_fill(&graph, None, true, counted),
        Some(vec![1, 1, 1, 1]),
        "no deadline and no stop is a pass that finishes",
    );
    assert_eq!(
        super::initial_fill(&graph, Some(past), true, counted),
        None,
        "an abortable pass stops at its deadline",
    );
    assert_eq!(
        super::initial_fill(&graph, Some(past), false, counted),
        Some(vec![1, 1, 1, 1]),
        "a pass that has to leave a complete decomposition runs to the end",
    );
}

#[test]
fn the_initial_fill_pass_counts_nothing_for_an_eliminated_vertex() {
    use crate::elimination::graph::EliminationGraph;

    let mut graph = EliminationGraph::from_edges(4, &[(0, 1), (1, 2), (2, 3), (0, 2)]);
    graph.active[1] = false;

    assert_eq!(
        super::initial_fill(&graph, None, true, |_| 7),
        Some(vec![7, 0, 7, 7]),
    );
}
