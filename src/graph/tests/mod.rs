use crate::Graph;
use crate::graph::{LocalIds, index_by_vertex, induced_edges};

#[test]
fn graph_new_canonicalizes_orientation_duplicates_order_and_self_loops() {
    let graph = Graph::new(5, [(3, 1), (1, 3), (4, 4), (0, 2), (3, 1)]);

    assert_eq!(graph.num_vertices(), 5);
    assert_eq!(graph.edges(), [(0, 2), (1, 3)]);
}

#[test]
fn a_subset_restriction_uses_subset_order_as_the_local_numbering() {
    let edges = [(2, 5), (5, 7), (2, 7), (1, 7), (7, 5)];
    let subset = [5, 2, 7];
    let mut local = LocalIds::new(8);

    assert_eq!(
        induced_edges(&edges, &subset, &mut local),
        vec![(0, 1), (0, 2), (1, 2)]
    );
    assert_eq!(index_by_vertex(&subset)[&5], 0);
    assert_eq!(index_by_vertex(&subset)[&2], 1);
    assert_eq!(index_by_vertex(&subset)[&7], 2);
}

#[test]
fn a_subset_restriction_drops_edges_with_an_endpoint_outside_the_subset() {
    let mut local = LocalIds::new(4);
    let restricted = induced_edges(&[(0, 1), (1, 2), (2, 3)], &[1, 2], &mut local);

    assert_eq!(restricted, vec![(0, 1)]);
}

#[test]
fn a_reused_scratch_index_restricts_the_same_way_every_time() {
    let edges = [(0, 1), (1, 2), (2, 3), (3, 4), (0, 4)];
    let mut local = LocalIds::new(5);

    // The scratch comes back unmarked, so a second subset that overlaps the
    // first sees only its own vertices.
    let first = induced_edges(&edges, &[1, 2, 3], &mut local);
    let second = induced_edges(&edges, &[4, 0], &mut local);
    let third = induced_edges(&edges, &[1, 2, 3], &mut local);

    assert_eq!(first, vec![(0, 1), (1, 2)]);
    assert_eq!(second, vec![(0, 1)]);
    assert_eq!(third, first);
}

#[test]
fn an_induced_subgraph_uses_the_requested_vertex_order() {
    let graph = Graph::new(8, [(2, 5), (5, 7), (2, 7), (1, 7)]);
    let induced = graph.induced_subgraph(&[5, 2, 7]).unwrap();

    assert_eq!(induced.num_vertices(), 3);
    assert_eq!(induced.edges(), [(0, 1), (0, 2), (1, 2)]);
}

#[test]
fn an_induced_subgraph_rejects_an_ambiguous_vertex_list() {
    let graph = Graph::new(4, []);

    assert!(graph.induced_subgraph(&[0, 0]).is_err());
    assert!(graph.induced_subgraph(&[4]).is_err());
}
