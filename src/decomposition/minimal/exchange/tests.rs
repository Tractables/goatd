use super::*;
use crate::elimination::{Order, decompose};
use std::time::Duration;

#[test]
fn a_cycle_diagonal_can_be_exchanged_with_branches_on_both_sides() {
    let graph = Graph::new(6, [(0, 2), (2, 1), (1, 3), (3, 0), (0, 4), (1, 5)]);
    let seed = TreeDecomposition::new_trusted(
        &graph,
        vec![vec![0, 1, 2], vec![0, 1, 3], vec![0, 4], vec![1, 5]],
        vec![(0, 1), (0, 2), (1, 3)],
    )
    .unwrap();
    seed.validate(&graph).unwrap();
    let next = flip(&graph, &seed, 0, 1).unwrap();
    next.validate(&graph).unwrap();
    assert_eq!(next.treewidth(), 2);
    assert!(
        !next
            .bags()
            .iter()
            .any(|b| b.vertices().contains(&0) && b.vertices().contains(&1))
    );
    assert!(
        next.bags()
            .iter()
            .any(|b| b.vertices().contains(&2) && b.vertices().contains(&3))
    );
    assert!(flip(&graph, &seed, 0, 2).is_none());
}

#[test]
fn all_five_vertex_graphs_keep_edge_coverage_and_running_intersection_after_every_flip() {
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
                .map(|(_, &e)| e),
        );
        let seed = decompose(&graph, Order::MinFill, 0, None).unwrap();
        for &(u, v) in &pairs {
            if let Some(next) = flip(&graph, &seed, u, v) {
                next.validate(&graph).unwrap();
                checked += 1;
            }
        }
        let options = candidates(
            &graph,
            &seed,
            seed.treewidth(),
            Instant::now() + Duration::from_secs(10),
        );
        for (u, v) in options {
            assert!(flip(&graph, &seed, u, v).unwrap().treewidth() <= seed.treewidth());
        }
    }
    assert!(checked > 100);
}

#[test]
fn an_expired_budget_retains_the_input() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3), (3, 0)]);
    let seed = decompose(&graph, Order::MinFill, 0, None).unwrap();
    let (next, stats) = improve(&graph, &seed, Instant::now()).unwrap();
    assert_eq!(
        next.to_td(),
        seed.subsumed_bag_compaction().apply(seed.clone()).to_td()
    );
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
fn every_edge_of_every_five_vertex_chordal_completion_can_be_exchanged() {
    let pairs: Vec<_> = (0..5)
        .flat_map(|u| (u + 1..5).map(move |v| (u, v)))
        .collect();
    let mut checked = 0;
    for mask in 0..(1usize << pairs.len()) {
        let edges: Vec<_> = pairs
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, &edge)| edge)
            .collect();
        let completion = Graph::new(5, edges.clone());
        let seed = decompose(&completion, Order::MinFill, 0, None).unwrap();
        let chordal = seed.bags().iter().all(|bag| {
            bag.vertices().iter().all(|&u| {
                bag.vertices()
                    .iter()
                    .all(|&v| u == v || edges.contains(&(u.min(v), u.max(v))))
            })
        });
        if !chordal {
            continue;
        }
        for &(u, v) in &edges {
            let graph = Graph::new(5, edges.iter().copied().filter(|&edge| edge != (u, v)));
            let next = flip(&graph, &seed, u, v).unwrap();
            next.validate(&graph).unwrap();
            assert!(
                !next
                    .bags()
                    .iter()
                    .any(|bag| bag.vertices().contains(&u) && bag.vertices().contains(&v))
            );
            checked += 1;
        }
    }
    assert!(checked > 1000);
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
