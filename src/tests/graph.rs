use crate::{Graph, local_index, restrict_to_subset};

#[test]
fn graph_new_canonicalizes_orientation_duplicates_order_and_self_loops() {
    let graph = Graph::new(5, [(3, 1), (1, 3), (4, 4), (0, 2), (3, 1)]);

    assert_eq!(graph.num_vertices, 5);
    assert_eq!(graph.edges, vec![(0, 2), (1, 3)]);
}

#[test]
fn a_subset_restriction_uses_subset_order_as_the_local_numbering() {
    let edges = [(2, 5), (5, 7), (2, 7), (1, 7), (7, 5)];
    let subset = [5, 2, 7];

    assert_eq!(
        restrict_to_subset(&edges, &subset),
        vec![(0, 1), (0, 2), (1, 2)],
    );
    assert_eq!(local_index(&subset)[&5], 0);
    assert_eq!(local_index(&subset)[&2], 1);
    assert_eq!(local_index(&subset)[&7], 2);
}

#[test]
fn a_subset_restriction_drops_edges_with_an_endpoint_outside_the_subset() {
    let restricted = restrict_to_subset(&[(0, 1), (1, 2), (2, 3)], &[1, 2]);

    assert_eq!(restricted, vec![(0, 1)]);
}
