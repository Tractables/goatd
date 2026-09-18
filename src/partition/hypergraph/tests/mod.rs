mod refine_fm;

use super::coarsen::coarsen_one_level;
use super::initial::{greedy_growing, hyperedge_cut};
use super::model::Hypergraph;
use super::refine_flow::{FlowNetwork, flow_refine};
use super::refine_fm::FmScratch;
use crate::partition::common::BisectionStop;
use crate::rng::Xorshift64;

#[test]
fn hypergraph_storage_indexes_pins_in_both_directions() {
    let hyperedges = vec![vec![0, 2], vec![1, 2, 3], vec![0, 3]];
    let hg = Hypergraph::from_hyperedges(5, &hyperedges, Some(&[2, 3, 5]));

    assert_eq!(hg.vertex_count, 5);
    assert_eq!(hg.num_hyperedges(), 3);
    assert_eq!(hg.hyperedge_weights, vec![2, 3, 5]);
    assert_eq!(hg.charged_hyperedge_pins(0), &[0, 2]);
    assert_eq!(hg.charged_hyperedge_pins(1), &[1, 2, 3]);
    assert_eq!(hg.vertex_hyperedges(0), &[0, 2]);
    assert_eq!(hg.vertex_hyperedges(2), &[0, 1]);
    assert!(hg.vertex_hyperedges(4).is_empty());
    let mut counts = Vec::new();
    hg.fill_pin_counts(&[0, 1, 0, 1, 0], &mut counts);
    assert_eq!(counts, vec![[2, 0], [1, 2], [1, 1]]);
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

    let part = greedy_growing(&hg, 3, &mut BisectionStop::new(None));
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

    assert_eq!(level.hg.vertex_count, 2);
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

    assert_eq!(
        network.max_flow(0, 3, &mut source_side, &mut BisectionStop::new(None)),
        3
    );
    assert_eq!(source_side, vec![true, false, false, false]);

    // A reset network is the network a fresh one would be: the arcs the last
    // build pushed are gone along with the flow they carried.
    network.reset(4);
    network.add_edge(0, 1, 5);
    network.add_edge(1, 3, 1);
    assert_eq!(
        network.max_flow(0, 3, &mut source_side, &mut BisectionStop::new(None)),
        1
    );
    assert_eq!(source_side, vec![true, true, false, false]);
}

#[test]
fn flow_refinement_declines_tiny_hypergraphs_without_changing_them() {
    let hg = Hypergraph::from_hyperedges(4, &[vec![0, 1, 2], vec![1, 3]], None);
    let mut part = vec![0, 0, 1, 1];

    assert!(!flow_refine(
        &hg,
        &mut part,
        0.25,
        &mut FmScratch::new(),
        &mut BisectionStop::new(None)
    ));
    assert_eq!(part, vec![0, 0, 1, 1]);
}

/// The corridor cap stops the pin walk part way, and the pins it does not walk
/// are charged all the same: a budgeted run repeats on the meter, so where the
/// walk stops must not show up in the units.
#[test]
fn a_corridor_over_the_cap_is_declined_at_the_same_charge() {
    // 600 two-pin hyperedges over 1,200 vertices, every one of them cut, so the
    // corridor passes the cap about two fifths of the way through.
    let hyperedges: Vec<Vec<u32>> = (0..600u32).map(|h| vec![2 * h, 2 * h + 1]).collect();
    let hg = Hypergraph::from_hyperedges(1_200, &hyperedges, None);
    let mut part: Vec<u8> = (0..1_200).map(|v| (v % 2) as u8).collect();
    let before = part.clone();

    let guard = crate::meter::arm(std::time::Instant::now());
    let start = crate::meter::units_spent();
    let refined = flow_refine(
        &hg,
        &mut part,
        0.25,
        &mut FmScratch::new(),
        &mut BisectionStop::new(None),
    );
    let spent = crate::meter::units_spent() - start;
    drop(guard);

    assert!(!refined);
    assert_eq!(part, before);
    // The counts walk every pin once, and the corridor charges for every pin of
    // every cut hyperedge whether it walked it or not.
    assert_eq!(spent, 2 * 1_200);
}

/// A corridor inside the vertex cap whose cut hyperedges are not: the same
/// twenty vertices carry twenty thousand of them, so the network the pass
/// would build has a node per hyperedge and two arcs, and nothing about the
/// corridor bounds it. It declines, and charges what it did not walk.
#[test]
fn a_network_over_the_arc_cap_is_declined_at_the_same_charge() {
    // Twenty thousand two-pin hyperedges over twenty vertices, every one of
    // them cut, so the arc count passes its cap four fifths of the way
    // through while the corridor stays at twenty.
    let hyperedges: Vec<Vec<u32>> = (0..20_000u32)
        .map(|h| vec![h % 20, (h % 20 + 1) % 20])
        .collect();
    let hg = Hypergraph::from_hyperedges(20, &hyperedges, None);
    let mut part: Vec<u8> = (0..20).map(|v| (v % 2) as u8).collect();
    let before = part.clone();

    let guard = crate::meter::arm(std::time::Instant::now());
    let start = crate::meter::units_spent();
    let refined = flow_refine(
        &hg,
        &mut part,
        0.25,
        &mut FmScratch::new(),
        &mut BisectionStop::new(None),
    );
    let spent = crate::meter::units_spent() - start;
    drop(guard);

    assert!(!refined);
    assert_eq!(part, before);
    // As for the corridor cap: the counts walk every pin once, and the
    // corridor charges for every pin of every cut hyperedge either way.
    assert_eq!(spent, 2 * 40_000);
}

/// The finest level's scratch outlives a pass: it is held across the levels of
/// a sweep and across the sweeps of a bisection, and the corridor is stamped
/// with a pass number rather than cleared. A scratch that has been used already
/// has to decide what a fresh one decides, at any size and in any order.
#[test]
fn a_used_scratch_decides_what_a_fresh_one_does() {
    let wide_pins: Vec<Vec<u32>> = (0..600u32).map(|h| vec![2 * h, 2 * h + 1]).collect();
    let wide = Hypergraph::from_hyperedges(1_200, &wide_pins, None);
    // Nineteen cut hyperedges over twenty vertices: a corridor under the cap,
    // so this one builds its network and runs the flow.
    let narrow_pins: Vec<Vec<u32>> = (0..19u32).map(|v| vec![v, v + 1]).collect();
    let narrow = Hypergraph::from_hyperedges(20, &narrow_pins, None);

    let mut held = FmScratch::new();
    for hg in [&wide, &narrow, &wide, &narrow] {
        let start: Vec<u8> = (0..hg.vertex_count).map(|v| (v % 2) as u8).collect();

        let mut fresh_part = start.clone();
        let fresh = flow_refine(
            hg,
            &mut fresh_part,
            0.25,
            &mut FmScratch::new(),
            &mut BisectionStop::new(None),
        );

        let mut held_part = start.clone();
        let again = flow_refine(
            hg,
            &mut held_part,
            0.25,
            &mut held,
            &mut BisectionStop::new(None),
        );

        assert_eq!(again, fresh, "{} vertices", hg.vertex_count);
        assert_eq!(held_part, fresh_part, "{} vertices", hg.vertex_count);
    }
}
