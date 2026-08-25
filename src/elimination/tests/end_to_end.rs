//! End-to-end tests of the elimination orders.

use std::time::Duration;

use crate::elimination::graph::Graph;
use crate::elimination::preprocess::preprocess;
use crate::elimination::width_opt::Config;
use crate::elimination::{ScheduleConfig, elimination_td, five_slot_schedule, refined_td};
use crate::tests::td_fixture::{GraphAtItsWidth, assert_valid_td};

use super::width_opt::run_config;

fn minfill(num_vertices: u32, edges: &[(u32, u32)]) -> crate::TreeDecomposition {
    run_config(num_vertices, edges, Config::MinFill, 0).td
}

#[test]
fn triangle_is_width_two() {
    let edges = vec![(0, 1), (0, 2), (1, 2)];
    let td = minfill(3, &edges);
    assert_eq!(td.treewidth(), 2);
    assert_valid_td(&td, 3, &edges);
}

#[test]
fn path_is_width_one() {
    let edges = vec![(0, 1), (1, 2), (2, 3), (3, 4)];
    let td = minfill(5, &edges);
    assert_eq!(td.treewidth(), 1);
    assert_valid_td(&td, 5, &edges);
}

#[test]
fn cycle_four_is_width_two() {
    // C_4 has treewidth 2; requires one fill edge.
    let edges = vec![(0, 1), (1, 2), (2, 3), (3, 0)];
    let td = minfill(4, &edges);
    assert_eq!(td.treewidth(), 2);
    assert_valid_td(&td, 4, &edges);
}

#[test]
fn three_tree_has_width_three() {
    // 3-tree on 5 vertices: start from K_4 {0,1,2,3}, then attach vertex 4
    // connected to three existing neighbours {0,1,2}.
    let edges = vec![
        (0, 1),
        (0, 2),
        (0, 3),
        (1, 2),
        (1, 3),
        (2, 3),
        (0, 4),
        (1, 4),
        (2, 4),
    ];
    let td = minfill(5, &edges);
    assert_eq!(td.treewidth(), 3);
    assert_valid_td(&td, 5, &edges);
}

#[test]
fn disconnected_forest_covered() {
    let edges = vec![(0, 1), (2, 3)];
    let td = minfill(4, &edges);
    assert_valid_td(&td, 4, &edges);
    assert!(td.treewidth() <= 1);
}

#[test]
fn preprocessing_preserves_covering() {
    // Series-reducible pentagon: preprocessing should completely eliminate
    // every vertex via series reductions.
    let edges = vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)];
    let graph = Graph::from_edges(5, &edges);
    let reduced = preprocess(graph);
    assert_eq!(reduced.prefix.bags.len(), 5);
    assert_eq!(reduced.graph.num_active, 0);
}

#[test]
fn multi_seed_produces_valid_tds() {
    let edges = vec![(0, 1), (0, 2), (1, 3), (2, 3), (3, 4)];
    for seed in 0..4u64 {
        let td = run_config(5, &edges, Config::MinFill, seed).td;
        assert_valid_td(&td, 5, &edges);
    }
}

/// The public entry points, on a graph with something for each of them to
/// do: a 4x4 grid, whose treewidth is 4.
#[test]
fn the_public_entry_points_decompose_a_grid() {
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for row in 0..4u32 {
        for col in 0..4u32 {
            let v = row * 4 + col;
            if col + 1 < 4 {
                edges.push((v, v + 1));
            }
            if row + 1 < 4 {
                edges.push((v, v + 4));
            }
        }
    }
    let graph = crate::Graph::new(16, edges.iter().copied());
    let weight = vec![1u32; 16];

    let single = elimination_td(&graph, Config::MinFill, 0, Some(Duration::from_secs(10)));
    assert_valid_td(&single, 16, &edges);
    assert!(single.treewidth() >= 4);

    let unbounded = elimination_td(&graph, Config::MinDegree, 0, None);
    assert_valid_td(&unbounded, 16, &edges);

    let tds = five_slot_schedule(&graph, &weight, 0, ScheduleConfig::five_slot(None));
    assert!(!tds.is_empty(), "slot 0 always produces a decomposition");
    for td in &tds {
        assert_valid_td(td, 16, &edges);
    }
    let keys: Vec<(u32, usize)> = tds
        .iter()
        .map(|td| (td.treewidth(), td.total_bag_size()))
        .collect();
    assert!(
        keys.windows(2).all(|w| w[0] <= w[1]),
        "the list is sorted by (width, total bag size): {keys:?}",
    );

    let refined = refined_td(&graph, &weight, 0, ScheduleConfig::five_slot(None), None);
    assert_valid_td(&refined, 16, &edges);
    assert!(
        (refined.treewidth(), refined.total_bag_size()) <= keys[0],
        "refinement never returns a worse decomposition than the winner",
    );
}

/// The refined path keys on `(width, total_bag_size)`: at equal width it
/// prefers the smaller total bag size, whatever else a caller might know
/// about the candidates.
#[test]
fn refined_key_orders_by_width_then_bagsize() {
    use crate::elimination::refined_select_key;
    let cands = [("small_bag", 5u32, 10usize), ("big_bag", 5, 20)];
    let winner = cands
        .iter()
        .min_by_key(|&&(_, w, b)| refined_select_key(w, b))
        .unwrap();
    assert_eq!(winner.0, "small_bag");
    assert!(refined_select_key(4, 100) < refined_select_key(5, 10));
}

/// The six smallest shapes a decomposer meets — one vertex, a path, a cycle, a
/// star, a complete graph and a forest with an isolated vertex — through every
/// elimination config there is.
///
/// The width bound is the shape's own treewidth, and no decomposition can go
/// below it, so an upper bound pins it exactly.
#[test]
fn every_elimination_config_decomposes_the_tiny_graph_family() {
    let shapes: &[GraphAtItsWidth] = &[
        ("one vertex", 1, &[], 0),
        ("a path", 5, &[(0, 1), (1, 2), (2, 3), (3, 4)], 1),
        ("a cycle", 4, &[(0, 1), (1, 2), (2, 3), (3, 0)], 2),
        ("a star", 4, &[(0, 1), (0, 2), (0, 3)], 1),
        (
            "a complete graph",
            4,
            &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)],
            3,
        ),
        ("a forest with an isolated vertex", 5, &[(0, 1), (2, 3)], 1),
    ];

    for &(shape, num_vertices, edges, treewidth) in shapes {
        let weight = vec![1u32; num_vertices as usize];
        let configs: [(&str, Config<'_>); 5] = [
            ("min-fill", Config::MinFill),
            ("min-degree", Config::MinDegree),
            ("nested dissection", Config::NestedDissection),
            (
                "sampled min-fill",
                Config::MinFillSampled { weight: &weight },
            ),
            (
                "sampled min-degree",
                Config::MinDegreeSampled { weight: &weight },
            ),
        ];
        for (config_name, config) in configs {
            let td = run_config(num_vertices, edges, config, 0).td;
            assert_valid_td(&td, num_vertices, edges);
            assert!(
                td.treewidth() <= treewidth,
                "{config_name} on {shape} must reach width {treewidth}, got {}",
                td.treewidth(),
            );
        }
    }
}

/// A sampling elimination breaks its ties by a seeded draw, so one seed has to
/// give one decomposition every time — otherwise a decomposition cannot be
/// reproduced from the seed that names it. Across seeds the draws differ, which
/// is what makes running several of them worth anything.
#[test]
fn a_seeded_sampling_elimination_repeats_its_decomposition() {
    // A 3x4 grid: symmetric enough that every step has a tie set to draw from,
    // and dense enough that preprocessing does not reduce it away first.
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for row in 0..3u32 {
        for col in 0..4u32 {
            let v = row * 4 + col;
            if col + 1 < 4 {
                edges.push((v, v + 1));
            }
            if row + 1 < 3 {
                edges.push((v, v + 4));
            }
        }
    }
    let weight: Vec<u32> = (0..12u32).map(|v| v + 1).collect();
    let sampled =
        |seed: u64| run_config(12, &edges, Config::MinFillSampled { weight: &weight }, seed).td;

    for seed in [0u64, 7, 99] {
        assert_eq!(
            sampled(seed).bags,
            sampled(seed).bags,
            "seed {seed} must draw the same decomposition twice",
        );
    }

    let first = sampled(0);
    assert!(
        (1..24u64).any(|seed| sampled(seed).bags != first.bags),
        "the seeds must not all collapse onto one decomposition",
    );
}
