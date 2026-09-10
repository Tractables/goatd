use super::*;
use crate::elimination::{Order, decompose};

#[test]
fn every_vertex_of_every_five_vertex_graph_can_be_rebuilt() {
    let pairs: Vec<_> = (0..5)
        .flat_map(|u| (u + 1..5).map(move |v| (u, v)))
        .collect();
    let mut checked = 0;
    for mask in 0..(1usize << pairs.len()) {
        let graph = Graph::new(
            5,
            pairs
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, &edge)| edge),
        );
        let seed = decompose(&graph, Order::MinFill, 0, None).unwrap();
        for vertex in 0..5 {
            let candidate = std::panic::catch_unwind(|| {
                rebuild(
                    &graph,
                    &seed,
                    vertex,
                    Instant::now() + Duration::from_secs(1),
                )
            })
            .unwrap_or_else(|_| {
                panic!(
                    "mask={mask} vertex={vertex} graph={:?} seed={}",
                    graph.edges(),
                    seed.to_td()
                )
            });
            if let Some(next) = candidate {
                next.validate(&graph).unwrap();
                checked += 1;
            }
        }
    }
    assert!(checked > 4000);
}

#[test]
fn reinsertion_connects_disjoint_neighbour_occurrences() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3)]);
    let tree = TreeDecomposition::new(
        &graph,
        [vec![0, 1], vec![1, 2], vec![2, 3]],
        [(0, 1), (1, 2)],
    )
    .unwrap();
    assert_eq!(support(&tree, &[true, false, false, true]), vec![true; 3]);
    assert_eq!(
        support(&tree, &[false, true, false, false]),
        vec![true, false, false]
    );
}

#[test]
fn an_expired_budget_retains_the_compacted_seed() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3), (3, 0)]);
    let tree = decompose(&graph, Order::MinFill, 0, None).unwrap();
    let (next, stats) = improve(&graph, &tree, Instant::now()).unwrap();
    assert_eq!(next.to_td(), compact(tree).to_td());
    assert_eq!(stats.tried, 0);
}

#[test]
fn an_articulation_vertex_joins_the_residual_forest_on_reinsertion() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);
    // Removing the middle vertex leaves a filled edge that minimalization
    // deletes, producing two separate bag trees.
    let tree = TreeDecomposition::new(&graph, [vec![0, 1, 2]], []).unwrap();
    let rebuilt = rebuild(&graph, &tree, 1, Instant::now() + Duration::from_secs(1)).unwrap();
    rebuilt.validate(&graph).unwrap();
    assert_eq!(rebuilt.treewidth(), 1);
}

#[test]
fn vertex_reconstruction_escapes_a_minimal_triangulation() {
    let graph = Graph::new(5, (0..2).flat_map(|u| (2..5).map(move |v| (u, v))));
    let tree =
        TreeDecomposition::new(&graph, [vec![0, 2, 3, 4], vec![1, 2, 3, 4]], [(0, 1)]).unwrap();
    // Completing the three-vertex side is minimal: removing any of its
    // fill edges leaves a chordless cycle through vertices 0 and 1.
    let mut filled = super::super::completion(&tree, 5, None).unwrap();
    assert_eq!(super::super::minimalize(&mut filled, &graph, 5, None), 0);
    assert_eq!(tree.treewidth(), 3);
    let (next, stats) = improve_trusted(&graph, &tree, Instant::now() + Duration::from_secs(1));
    next.validate(&graph).unwrap();
    assert_eq!(next.treewidth(), 2);
    assert!(stats.improved > 0);
}

#[test]
fn the_checked_entry_rejects_a_tree_for_a_different_graph() {
    let graph = Graph::new(2, []);
    let tree = TreeDecomposition::new(&graph, [vec![0], vec![1]], [(0, 1)]).unwrap();
    assert!(improve(&Graph::new(2, [(0, 1)]), &tree, Instant::now()).is_err());
}

#[test]
fn direct_reinsertion_matches_completed_edges_and_exact_quality_on_small_graphs() {
    let pairs: Vec<_> = (0..5)
        .flat_map(|u| (u + 1..5).map(move |v| (u, v)))
        .collect();
    let mut checked = 0;
    for mask in 0..(1usize << pairs.len()) {
        let graph = Graph::new(
            5,
            pairs
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, &edge)| edge),
        );
        let seeds = [
            decompose(&graph, Order::MinFill, 0, None).unwrap(),
            TreeDecomposition::new(&graph, [vec![0, 1, 2, 3, 4]], []).unwrap(),
        ];
        for seed in seeds {
            for vertex in 0..5 {
                let deadline = Instant::now() + Duration::from_secs(2);
                let full = rebuild(&graph, &seed, vertex, deadline);
                let direct = std::panic::catch_unwind(|| {
                    rebuild_candidate::<true>(&graph, &seed, vertex, deadline)
                })
                .unwrap_or_else(|_| {
                    panic!("mask={mask} vertex={vertex} seed={}", seed.to_td());
                });
                assert_eq!(full.is_some(), direct.is_some());
                if let (Some(full), Some(direct)) = (full, direct) {
                    direct.validate(&graph).unwrap();
                    let a = super::super::completion(&full, 5, None).unwrap();
                    let b = super::super::completion(&direct, 5, None).unwrap();
                    assert_eq!(
                        a.rows,
                        b.rows,
                        "mask={mask} vertex={vertex} seed={}",
                        seed.to_td()
                    );
                    assert_eq!(
                        quality(&full),
                        quality(&direct),
                        "mask={mask} vertex={vertex}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 9600);
}

#[test]
fn connecting_bags_avoid_private_vertices_of_a_large_bag() {
    let graph = Graph::new(
        5,
        (1..5)
            .flat_map(|a| (a + 1..5).map(move |b| (a, b)))
            .chain([(0, 1), (0, 2)]),
    );
    let seed = TreeDecomposition::new(&graph, [vec![0, 1, 2, 3, 4]], []).unwrap();
    let direct =
        rebuild_candidate::<true>(&graph, &seed, 0, Instant::now() + Duration::from_secs(1))
            .unwrap();
    direct.validate(&graph).unwrap();
    assert_eq!(direct.treewidth(), 3);
    assert_eq!(direct.bags().len(), 2);
    let filled = super::super::completion(&direct, 5, None).unwrap();
    assert!(!filled.contains(0, 3));
    assert!(!filled.contains(0, 4));
    assert_eq!(
        quality(&direct),
        quality(
            &TreeDecomposition::new(&graph, [vec![1, 2, 3, 4], vec![0, 1, 2]], [(0, 1)]).unwrap()
        )
    );
}

#[test]
fn connecting_bags_join_the_needed_components_of_a_residual_forest() {
    let graph = Graph::new(4, [(0, 1), (1, 2)]);
    let seed = TreeDecomposition::new(&graph, [vec![0, 1, 2], vec![3]], []).unwrap();
    let direct =
        rebuild_candidate::<true>(&graph, &seed, 1, Instant::now() + Duration::from_secs(1))
            .unwrap();
    direct.validate(&graph).unwrap();
    assert_eq!(direct.treewidth(), 1);
    assert_eq!(direct.bags().len(), 3);
}

#[test]
fn expired_direct_search_retains_the_seed() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3), (3, 0)]);
    let seed = decompose(&graph, Order::MinFill, 0, None).unwrap();
    let (next, stats) = improve_direct_trusted(&graph, &seed, Instant::now());
    assert_eq!(next.to_td(), compact(seed).to_td());
    assert_eq!(stats.tried, 0);
}
