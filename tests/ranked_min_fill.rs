use goatd::{
    Graph,
    elimination::{self, Order},
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
