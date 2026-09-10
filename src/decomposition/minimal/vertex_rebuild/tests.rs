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
fn the_checked_entry_rejects_a_tree_for_a_different_graph() {
    let graph = Graph::new(2, []);
    let tree = TreeDecomposition::new(&graph, [vec![0], vec![1]], [(0, 1)]).unwrap();
    assert!(improve(&Graph::new(2, [(0, 1)]), &tree, Instant::now()).is_err());
}
