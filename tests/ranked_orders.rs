use goatd::{
    Graph,
    elimination::{self, Order, Prepared, RunConfig, RunOutcome},
};

/// The cube: every vertex has degree three and fill three, no reduction
/// applies, and the first step is a tie among all eight vertices.
fn cube() -> Graph {
    let edges = (0..8u32).flat_map(|v| {
        [1, 2, 4]
            .into_iter()
            .map(move |bit| (v, v ^ bit))
            .filter(|&(u, w)| u < w)
    });
    Graph::new(8, edges)
}

#[test]
fn ranked_min_fill_breaks_the_first_tie_by_rank_for_every_seed() {
    let graph = cube();
    for first in [0u32, 5, 7] {
        let ranks: Vec<u32> = (0..8).map(|v| if v == first { 0 } else { 1 + v }).collect();
        let reference =
            elimination::decompose(&graph, Order::MinFillRanked { ranks: &ranks }, 0, None)
                .unwrap();
        reference.validate(&graph).unwrap();
        // The first step's bag is the first vertex and its three neighbours.
        let mut bag = reference.bags()[0].vertices().to_vec();
        bag.sort_unstable();
        let mut expected = vec![first, first ^ 1, first ^ 2, first ^ 4];
        expected.sort_unstable();
        assert_eq!(bag, expected);
        for seed in [1, 9] {
            let again =
                elimination::decompose(&graph, Order::MinFillRanked { ranks: &ranks }, seed, None)
                    .unwrap();
            assert_eq!(again.bags(), reference.bags());
        }
    }
}

#[test]
fn ranked_min_fill_rejects_a_rank_count_that_does_not_match_the_graph() {
    let graph = cube();
    let ranks = vec![0u32; 7];
    assert!(
        elimination::decompose(&graph, Order::MinFillRanked { ranks: &ranks }, 0, None).is_err()
    );
}

fn bag_sets(tree: &goatd::TreeDecomposition) -> Vec<Vec<u32>> {
    tree.bags()
        .iter()
        .map(|bag| {
            let mut vertices = bag.vertices().to_vec();
            vertices.sort_unstable();
            vertices
        })
        .collect()
}

#[test]
fn an_unreduced_ranked_order_eliminates_every_vertex_in_rank_order() {
    // The path 0-1-2-3-4, eliminated 2, 0, 4, 1, 3. The reductions would
    // take the twigs 0 and 4 first.
    let graph = Graph::new(5, [(0, 1), (1, 2), (2, 3), (3, 4)]);
    let ranks = [1, 3, 0, 4, 2];
    let mut prepared = Prepared::unreduced(&graph);
    let RunOutcome::Completed(tree) = prepared
        .run(Order::Ranked { ranks: &ranks }, 0, RunConfig::default())
        .unwrap()
    else {
        panic!("an unbounded run completes");
    };
    tree.validate(&graph).unwrap();
    assert_eq!(
        bag_sets(&tree),
        vec![vec![1, 2, 3], vec![0, 1], vec![3, 4], vec![1, 3], vec![3]]
    );
    // Eliminating 2 first makes a bag of width 2.
    let bounded = prepared
        .run(
            Order::Ranked { ranks: &ranks },
            0,
            RunConfig::default().with_width_bound(Some(1)),
        )
        .unwrap();
    assert_eq!(bounded, RunOutcome::WidthAborted);
}

#[test]
fn an_unreduced_ranked_min_fill_still_breaks_ties_by_rank() {
    let graph = cube();
    let ranks: Vec<u32> = (0..8).rev().collect();
    let mut prepared = Prepared::unreduced(&graph);
    let RunOutcome::Completed(tree) = prepared
        .run(
            Order::MinFillRanked { ranks: &ranks },
            3,
            RunConfig::default(),
        )
        .unwrap()
    else {
        panic!("an unbounded run completes");
    };
    tree.validate(&graph).unwrap();
    assert_eq!(bag_sets(&tree)[0], vec![3, 5, 6, 7]);
}
