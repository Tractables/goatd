//! End-to-end tests of the elimination orders.

use std::time::Duration;

use crate::elimination::Order;
use crate::elimination::decompose;
use crate::elimination::graph::EliminationGraph;
use crate::elimination::preprocess::preprocess;
use crate::tests::td_fixture::{GraphAtItsWidth, assert_valid_td};

use super::engine::run_order;

fn minfill(num_vertices: u32, edges: &[(u32, u32)]) -> crate::TreeDecomposition {
    run_order(num_vertices, edges, Order::MinFill, 0)
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
    let graph = EliminationGraph::from_edges(5, &edges);
    let reduced = preprocess(graph);
    assert_eq!(reduced.prefix.bags.len(), 5);
    assert_eq!(reduced.graph.num_active, 0);
}

#[test]
fn multi_seed_produces_valid_tds() {
    let edges = vec![(0, 1), (0, 2), (1, 3), (2, 3), (3, 4)];
    for seed in 0..4u64 {
        let td = run_order(5, &edges, Order::MinFill, seed);
        assert_valid_td(&td, 5, &edges);
    }
}

#[test]
fn every_elimination_config_decomposes_every_graph_through_five_vertices() {
    for num_vertices in 0..=5u32 {
        let possible_edges: Vec<(u32, u32)> = (0..num_vertices)
            .flat_map(|u| ((u + 1)..num_vertices).map(move |v| (u, v)))
            .collect();
        for mask in 0..(1u64 << possible_edges.len()) {
            let mut edges: Vec<(u32, u32)> = Vec::new();
            for (bit, &edge) in possible_edges.iter().enumerate() {
                if (mask >> bit) & 1 == 1 {
                    edges.push(edge);
                }
            }
            let graph = crate::Graph::new(num_vertices, edges.iter().copied());
            let weights = vec![1; num_vertices as usize];
            let orders = [
                Order::MinFill,
                Order::MinDegree,
                Order::NestedDissection,
                Order::MinFillSampled { weights: &weights },
                Order::MinDegreeSampled { weights: &weights },
                Order::FillDegreeSampled {
                    weights: &weights,
                    degree_coefficient: 1,
                },
                Order::FillDegreeSampled {
                    weights: &weights,
                    degree_coefficient: -16,
                },
            ];

            for order in orders {
                let td = decompose(&graph, order, 17, None).unwrap();
                td.validate(&graph).unwrap_or_else(|error| {
                    panic!(
                        "{order:?} returned an invalid decomposition of n={num_vertices}, \
                         mask={mask:#x}: {error}"
                    )
                });
            }
        }
    }
}

/// The public elimination entry point on a graph with enough structure for
/// both greedy orders to do work: a 4x4 grid, whose treewidth is 4.
#[test]
fn the_public_elimination_entry_point_decomposes_a_grid() {
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
    let single = decompose(&graph, Order::MinFill, 0, Some(Duration::from_secs(10))).unwrap();
    assert_valid_td(&single, 16, &edges);
    assert!(single.treewidth() >= 4);

    let unbounded = decompose(&graph, Order::MinDegree, 0, None).unwrap();
    assert_valid_td(&unbounded, 16, &edges);
}

/// The six smallest shapes a decomposer meets — one vertex, a path, a cycle, a
/// star, a complete graph and a forest with an isolated vertex — through every
/// elimination order there is.
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
        let weights = vec![1u32; num_vertices as usize];
        let orders: [(&str, Order<'_>); 7] = [
            ("min-fill", Order::MinFill),
            ("min-degree", Order::MinDegree),
            ("nested dissection", Order::NestedDissection),
            (
                "sampled min-fill",
                Order::MinFillSampled { weights: &weights },
            ),
            (
                "sampled min-degree",
                Order::MinDegreeSampled { weights: &weights },
            ),
            (
                "sampled fill-plus-degree",
                Order::FillDegreeSampled {
                    weights: &weights,
                    degree_coefficient: 1,
                },
            ),
            (
                "sampled fill-minus-degree",
                Order::FillDegreeSampled {
                    weights: &weights,
                    degree_coefficient: -1,
                },
            ),
        ];
        for (order_name, order) in orders {
            let td = run_order(num_vertices, edges, order, 0);
            assert_valid_td(&td, num_vertices, edges);
            assert!(
                td.treewidth() <= treewidth,
                "{order_name} on {shape} must reach width {treewidth}, got {}",
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
    let weights: Vec<u32> = (0..12u32).map(|v| v + 1).collect();
    let sampled = |seed: u64| {
        run_order(
            12,
            &edges,
            Order::MinFillSampled { weights: &weights },
            seed,
        )
    };

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
