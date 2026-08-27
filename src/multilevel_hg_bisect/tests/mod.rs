use super::coarsen::hg_coarsen_one_level;
use super::graph::Hypergraph;
use super::initial::{hg_cut, hg_greedy_growing};
use super::refine_flow::{edmonds_karp, hg_flow_refine};
use crate::Xorshift64;

#[test]
fn hypergraph_storage_indexes_pins_in_both_directions() {
    let hyperedges = vec![vec![0, 2], vec![1, 2, 3], vec![0, 3]];
    let hg = Hypergraph::from_hyperedges(5, &hyperedges, Some(&[2, 3, 5]));

    assert_eq!(hg.num_vertices, 5);
    assert_eq!(hg.num_hyperedges(), 3);
    assert_eq!(hg.hewgt, vec![2, 3, 5]);
    assert_eq!(hg.hyperedge_pins(0), &[0, 2]);
    assert_eq!(hg.hyperedge_pins(1), &[1, 2, 3]);
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

    assert_eq!(hg_cut(&hg, &[0, 0, 0, 0]), 0);
    assert_eq!(hg_cut(&hg, &[0, 0, 1, 0]), 7);
    assert_eq!(hg_cut(&hg, &[0, 0, 1, 1]), 18);
}

#[test]
fn greedy_growing_uses_the_seed_and_stops_at_half_the_vertex_weight() {
    let hg = Hypergraph::from_hyperedges(5, &[vec![0, 1, 2], vec![2, 3, 4]], None);

    let part = hg_greedy_growing(&hg, 3);
    assert_eq!(part[3], 0);
    assert_eq!(part.iter().filter(|&&side| side == 0).count(), 2);
    assert!(part.contains(&0) && part.contains(&1));
}

#[test]
fn coarsening_contracts_connected_pairs_and_sums_vertex_weight() {
    let hg = Hypergraph::from_hyperedges(4, &[vec![0, 1], vec![2, 3]], None);
    let mut rng = Xorshift64::from_state(9);
    let level = hg_coarsen_one_level(&hg, 0, &mut rng, None)
        .expect("two disjoint pairs contract to two vertices");

    assert_eq!(level.hg.num_vertices, 2);
    assert_eq!(level.mapping[0], level.mapping[1]);
    assert_eq!(level.mapping[2], level.mapping[3]);
    assert_ne!(level.mapping[0], level.mapping[2]);
    assert_eq!(level.hg.vwgt, vec![2, 2]);
    assert_eq!(level.hg.num_hyperedges(), 0);
}

#[test]
fn edmonds_karp_returns_the_flow_and_the_residual_source_side() {
    // 0 -> 1 -> 3 carries 2; 0 -> 2 -> 3 carries 1.
    let adj = vec![
        vec![(1, 0), (2, 4)],
        vec![(0, 1), (3, 2)],
        vec![(0, 5), (3, 6)],
        vec![(1, 3), (2, 7)],
    ];
    let mut capacity = vec![2, 0, 2, 0, 1, 0, 1, 0];
    let mut source_side = vec![false; 4];

    assert_eq!(
        edmonds_karp(4, &adj, &mut capacity, 0, 3, &mut source_side),
        3,
    );
    assert_eq!(source_side, vec![true, false, false, false]);
}

#[test]
fn flow_refinement_declines_tiny_hypergraphs_without_changing_them() {
    let hg = Hypergraph::from_hyperedges(4, &[vec![0, 1, 2], vec![1, 3]], None);
    let mut part = vec![0, 0, 1, 1];

    assert!(!hg_flow_refine(&hg, &mut part, 0.25));
    assert_eq!(part, vec![0, 0, 1, 1]);
}
