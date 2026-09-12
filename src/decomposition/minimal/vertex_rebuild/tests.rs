use super::*;
use crate::elimination::{Order, decompose};

fn completed(graph: &Graph, tree: &TreeDecomposition) -> SharedCompletion {
    let mut shared = SharedCompletion::new(graph);
    shared.complete(tree);
    shared
}

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
        for seed in [
            decompose(&graph, Order::MinFill, 0, None).unwrap(),
            TreeDecomposition::new(&graph, [vec![0, 1, 2, 3, 4]], []).unwrap(),
        ] {
            for vertex in 0..5 {
                let candidate = std::panic::catch_unwind(|| {
                    rebuild(
                        &graph,
                        &adjacency(&graph)[vertex as usize],
                        &mut completed(&graph, &seed),
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
    }
    assert_eq!(checked, 9600);
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
    let rebuilt = rebuild(
        &graph,
        &adjacency(&graph)[1],
        &mut completed(&graph, &tree),
        &tree,
        1,
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();
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
    let original = super::super::original_edges(&graph);
    assert_eq!(
        super::super::minimalize(
            &mut filled,
            5,
            graph.edges().len(),
            |row, word| original.row(row)[word],
            &mut super::super::NoWitnesses,
            None
        ),
        0
    );
    assert_eq!(tree.treewidth(), 3);
    let (next, stats) = improve_trusted(&graph, &tree, Instant::now() + Duration::from_secs(1));
    next.validate(&graph).unwrap();
    assert_eq!(next.treewidth(), 2);
    assert!(stats.improved > 0);
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
    let direct = rebuild(
        &graph,
        &adjacency(&graph)[0],
        &mut completed(&graph, &seed),
        &seed,
        0,
        Instant::now() + Duration::from_secs(1),
    )
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
    let direct = rebuild(
        &graph,
        &adjacency(&graph)[1],
        &mut completed(&graph, &seed),
        &seed,
        1,
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();
    direct.validate(&graph).unwrap();
    assert_eq!(direct.treewidth(), 1);
    assert_eq!(direct.bags().len(), 3);
}

#[test]
fn expired_direct_search_retains_the_seed() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3), (3, 0)]);
    let seed = decompose(&graph, Order::MinFill, 0, None).unwrap();
    let (next, stats) = improve_trusted(&graph, &seed, Instant::now());
    assert_eq!(next.to_td(), compact(seed).to_td());
    assert_eq!(stats.tried, 0);
}

#[test]
fn mass_comparison_is_exact_across_limbs_and_bag_orders() {
    let graph = Graph::new(130, []);
    let mut bags = vec![(0..130).collect::<Vec<u32>>()];
    bags.extend((0..64).map(|_| (0..64).collect::<Vec<u32>>()));
    let edges: Vec<_> = (1..bags.len()).map(|i| (0, i)).collect();
    let tree = TreeDecomposition::new_trusted(&graph, bags.clone(), edges.clone()).unwrap();
    bags[1].push(64);
    let larger = TreeDecomposition::new_trusted(&graph, bags, edges).unwrap();
    assert!(quality(&tree) < quality(&larger));
    assert_eq!(quality(&tree).2, vec![4, 64, 0]);
}

#[test]
fn redundant_elimination_bags_do_not_distort_the_search_objective() {
    let graph = Graph::new(
        11,
        [
            (0, 2),
            (0, 6),
            (0, 7),
            (0, 8),
            (0, 9),
            (0, 10),
            (1, 4),
            (1, 7),
            (2, 3),
            (2, 6),
            (2, 7),
            (3, 7),
            (3, 9),
            (3, 10),
            (6, 9),
            (7, 9),
            (7, 10),
            (8, 9),
        ],
    );
    let tree = decompose(&graph, Order::MinFill, 0, None).unwrap();
    let compact = tree.subsumed_bag_compaction().apply(tree.clone());
    assert_eq!(quality(&compact), (4, 1, vec![82]));
    let (next, _) = improve(&graph, &tree, Instant::now() + Duration::from_secs(1)).unwrap();
    next.validate(&graph).unwrap();
    assert!(quality(&next) <= quality(&compact));
    assert_eq!(
        next.total_bag_size(),
        next.subsumed_bag_compaction().total_bag_size()
    );
}

#[test]
fn mass_uses_one_bag_for_a_chain_of_equal_bags() {
    let graph = Graph::new(2, [(0, 1)]);
    let seed = TreeDecomposition::new(&graph, vec![vec![0, 1]; 3], [(0, 2), (2, 1)]).unwrap();
    let (next, _) = improve(&graph, &seed, Instant::now()).unwrap();
    next.validate(&graph).unwrap();
    assert_eq!(next.bags().len(), 1);
    assert_eq!(quality(&next), (1, 1, vec![4]));
}
