use super::coarsen::coarsen_one_level;
use super::initial::{greedy_growing, hyperedge_cut};
use super::model::Hypergraph;
use super::refine_flow::{FlowNetwork, flow_refine};
use crate::rng::Xorshift64;

#[test]
fn hypergraph_storage_indexes_pins_in_both_directions() {
    let hyperedges = vec![vec![0, 2], vec![1, 2, 3], vec![0, 3]];
    let hg = Hypergraph::from_hyperedges(5, &hyperedges, Some(&[2, 3, 5]));

    assert_eq!(hg.num_vertices, 5);
    assert_eq!(hg.num_hyperedges(), 3);
    assert_eq!(hg.hyperedge_weights, vec![2, 3, 5]);
    assert_eq!(hg.charged_hyperedge_pins(0), &[0, 2]);
    assert_eq!(hg.charged_hyperedge_pins(1), &[1, 2, 3]);
    assert_eq!(hg.vertex_hyperedges(0), &[0, 2]);
    assert_eq!(hg.vertex_hyperedges(2), &[0, 1]);
    assert!(hg.vertex_hyperedges(4).is_empty());
    assert_eq!(
        hg.pin_counts(&[0, 1, 0, 1, 0]),
        vec![[2, 0], [1, 2], [1, 1]]
    );
}

#[test]
fn a_weighted_hyperedge_is_charged_once_when_cut() {
    let hg = Hypergraph::from_hyperedges(4, &[vec![0, 1, 2], vec![1, 3]], Some(&[7, 11]));

    assert_eq!(hyperedge_cut(&hg, &[0, 0, 0, 0]), 0);
    assert_eq!(hyperedge_cut(&hg, &[0, 0, 1, 0]), 7);
    assert_eq!(hyperedge_cut(&hg, &[0, 0, 1, 1]), 18);
}

#[test]
fn greedy_growing_uses_the_seed_and_stops_at_half_the_vertex_weight() {
    let hg = Hypergraph::from_hyperedges(5, &[vec![0, 1, 2], vec![2, 3, 4]], None);

    let part = greedy_growing(&hg, 3);
    assert_eq!(part[3], 0);
    assert_eq!(part.iter().filter(|&&side| side == 0).count(), 2);
    assert!(part.contains(&0) && part.contains(&1));
}

#[test]
fn coarsening_contracts_connected_pairs_and_sums_vertex_weight() {
    let hg = Hypergraph::from_hyperedges(4, &[vec![0, 1], vec![2, 3]], None);
    let mut rng = Xorshift64::from_state(9);
    let level = coarsen_one_level(&hg, 0, &mut rng, None)
        .expect("two disjoint pairs contract to two vertices");

    assert_eq!(level.hg.num_vertices, 2);
    assert_eq!(level.mapping[0], level.mapping[1]);
    assert_eq!(level.mapping[2], level.mapping[3]);
    assert_ne!(level.mapping[0], level.mapping[2]);
    assert_eq!(level.hg.vertex_weights, vec![2, 2]);
    assert_eq!(level.hg.num_hyperedges(), 0);
}

#[test]
fn coarsening_matches_across_the_heaviest_shared_hyperedge() {
    let hg = Hypergraph::from_hyperedges(
        4,
        &[vec![0, 1], vec![0, 2], vec![1, 3], vec![2, 3]],
        Some(&[1, 10, 10, 1]),
    );
    let mut rng = Xorshift64::from_state(11);
    let level = coarsen_one_level(&hg, 0, &mut rng, None)
        .expect("the four vertices must contract into two pairs");

    assert_eq!(level.mapping[0], level.mapping[2]);
    assert_eq!(level.mapping[1], level.mapping[3]);
    assert_ne!(level.mapping[0], level.mapping[1]);
}

#[test]
fn a_flow_network_returns_the_maximum_flow_and_residual_source_side() {
    // 0 -> 1 -> 3 carries 2; 0 -> 2 -> 3 carries 1.
    let mut network = FlowNetwork::new(4);
    network.add_edge(0, 1, 2);
    network.add_edge(1, 3, 2);
    network.add_edge(0, 2, 1);
    network.add_edge(2, 3, 1);
    let mut source_side = vec![false; 4];

    assert_eq!(network.max_flow(0, 3, &mut source_side), 3);
    assert_eq!(source_side, vec![true, false, false, false]);
}

#[test]
fn flow_refinement_declines_tiny_hypergraphs_without_changing_them() {
    let hg = Hypergraph::from_hyperedges(4, &[vec![0, 1, 2], vec![1, 3]], None);
    let mut part = vec![0, 0, 1, 1];

    assert!(!flow_refine(&hg, &mut part, 0.25));
    assert_eq!(part, vec![0, 0, 1, 1]);
}
