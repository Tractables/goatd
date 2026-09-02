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
    let mut buckets = super::BucketMap::with_weights(&weights);
    buckets.insert(0, 3);
    buckets.insert(1, 3);
    buckets.insert(2, 3);

    let (_, vertices, total_mass) = buckets.min_bucket().unwrap();
    assert_eq!(vertices, &[0, 1, 2]);
    assert_eq!(
        total_mass,
        super::sampling_mass(weights[0])
            + super::sampling_mass(weights[1])
            + super::sampling_mass(weights[2])
    );

    buckets.update(1, 7);
    buckets.remove_vertex(0);
    let (key, vertices, total_mass) = buckets.min_bucket().unwrap();
    assert_eq!(key, 3);
    assert_eq!(vertices, &[2]);
    assert_eq!(total_mass, super::sampling_mass(weights[2]));
}

#[test]
fn affected_membership_uses_one_word_per_vertex_block() {
    let affected = super::FillAffected::new(130);
    assert_eq!(affected.inside.len(), 3);
}

#[test]
fn affected_membership_excludes_the_eliminated_neighbourhood() {
    let mut graph = crate::elimination::graph::EliminationGraph::from_edges(
        130,
        &[(1, 2), (1, 65), (2, 65), (1, 129), (2, 129)],
    );
    graph.promote_bitset();
    let mut affected = super::FillAffected::new(130);

    assert!(affected.collect_deltas(&graph, &[1, 2, 65], &[(1, 2)], None));
    assert_eq!(affected.pop_delta(), Some((129, 1)));
    assert_eq!(affected.pop_delta(), None);
}
