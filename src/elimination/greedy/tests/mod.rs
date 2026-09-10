//! Beside the module: these drive the private elimination sink directly.

mod min_fill;

#[test]
fn sampling_mass_prefers_smaller_public_weights() {
    assert_eq!(super::sampling_mass(0), u64::from(u32::MAX) + 1);
    assert_eq!(super::sampling_mass(u32::MAX), 1);
    assert!(super::sampling_mass(7) > super::sampling_mass(8));
}

#[test]
fn uniform_sampling_repeats_the_generic_weighted_choices() {
    let weights = vec![7; 64];
    let vertices: Vec<u32> = (0..64).collect();
    let uniform_mass = super::uniform_sampling_mass(&weights).expect("equal weights");
    let mut fast = crate::rng::Xorshift64::from_state(17);
    let mut generic = fast;

    for len in 2..=vertices.len() {
        let total_mass = uniform_mass * len as u64;
        assert_eq!(
            super::sample_tie_set(
                &vertices[..len],
                &weights,
                &mut fast,
                Some(uniform_mass),
                total_mass,
            ),
            super::sample_tie_set(&vertices[..len], &weights, &mut generic, None, total_mass,),
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

    let mut scratch = Vec::new();
    let (vertices, total_mass) = buckets.min_band(0, &mut scratch).unwrap();
    assert_eq!(vertices, &[0, 1, 2]);
    assert_eq!(
        total_mass,
        super::sampling_mass(weights[0])
            + super::sampling_mass(weights[1])
            + super::sampling_mass(weights[2])
    );

    buckets.update(1, 7);
    buckets.remove_vertex(0);
    assert_eq!(buckets.minimum(), Some(3));
    let (vertices, total_mass) = buckets.min_band(0, &mut scratch).unwrap();
    assert_eq!(vertices, &[2]);
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
    let mut scratch = Vec::new();
    assert_eq!(buckets.min_band(0, &mut scratch).unwrap().1, 2 * mass);
    buckets.remove_vertex(0);
    assert_eq!(buckets.min_band(0, &mut scratch).unwrap().1, mass);
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
    let mut scratch = Vec::new();
    let (vertices, _) = buckets.min_band(0, &mut scratch).unwrap();
    assert_eq!(vertices, &[1]);
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
    let mut scratch = Vec::new();

    assert_eq!(buckets.min_band(0, &mut scratch).unwrap(), (&[0][..], mass));
    let (vertices, total_mass) = buckets.min_band(1, &mut scratch).unwrap();
    assert_eq!(
        vertices,
        &[0, 1, 2],
        "ascending by key, storage order inside"
    );
    assert_eq!(total_mass, 3 * mass);
    // Key 5 to 8 hold nothing, and the band stops before key 9.
    assert_eq!(buckets.min_band(5, &mut scratch).unwrap().0, &[0, 1, 2]);
    assert_eq!(buckets.min_band(6, &mut scratch).unwrap().0, &[0, 1, 2, 3]);
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
    let mut scratch = Vec::new();

    let (vertices, total_mass) = buckets.min_band(1, &mut scratch).unwrap();
    assert_eq!(vertices, &[0, 1]);
    assert_eq!(total_mass, 2 * mass);
    // The top of the band saturates instead of wrapping past u64::MAX.
    assert_eq!(buckets.min_band(4, &mut scratch).unwrap().0, &[0, 1]);
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
        let v = live[(rng.next_u64() % live.len() as u64) as usize];
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
