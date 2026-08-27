use crate::multilevel_bisect::multilevel_bisect;
use crate::multilevel_hg_bisect::multilevel_hg_bisect;

fn assert_two_nonempty_sides(part: &[u8], n: usize) {
    assert_eq!(part.len(), n);
    assert!(part.iter().all(|&side| side <= 1));
    if n >= 2 {
        assert!(part.contains(&0));
        assert!(part.contains(&1));
    }
}

#[test]
fn both_public_bisectors_handle_the_three_tiny_vertex_counts() {
    for n in 0..=2 {
        assert_eq!(multilevel_bisect(n, &[], 0.2, 7), &vec![0, 1][..n]);
        assert_eq!(
            multilevel_hg_bisect(n, &[], None, 0.2, 7, 1.0),
            &vec![0, 1][..n],
        );
    }
}

#[test]
fn an_edgeless_graph_and_hypergraph_still_split_both_sides() {
    let graph_part = multilevel_bisect(7, &[], 0.2, 3);
    let hypergraph_part = multilevel_hg_bisect(7, &[], None, 0.2, 3, 1.0);

    assert_two_nonempty_sides(&graph_part, 7);
    assert_two_nonempty_sides(&hypergraph_part, 7);
}

#[test]
fn the_public_bisectors_repeat_for_one_seed() {
    let edges = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 4),
        (4, 5),
        (5, 0),
        (0, 3),
        (1, 4),
        (2, 5),
    ];
    let hyperedges = vec![vec![0, 1, 2], vec![2, 3, 4], vec![0, 4, 5]];
    let weights = [2, 1, 3];

    let graph = multilevel_bisect(6, &edges, 0.2, 99);
    let hypergraph = multilevel_hg_bisect(6, &hyperedges, Some(&weights), 0.2, 99, 1.0);
    assert_eq!(graph, multilevel_bisect(6, &edges, 0.2, 99));
    assert_eq!(
        hypergraph,
        multilevel_hg_bisect(6, &hyperedges, Some(&weights), 0.2, 99, 1.0),
    );
    assert_two_nonempty_sides(&graph, 6);
    assert_two_nonempty_sides(&hypergraph, 6);
}
