//! Tests of the graph partitioner's private scoring representation.

mod initial;
mod refine_fm;

use super::csr::build_csr;
use super::{MAX_BISECTION_EDGES, validate_size};
use crate::Error;

#[test]
fn csr_size_guard_bounds_the_directed_arc_count() {
    validate_size(MAX_BISECTION_EDGES).expect("the documented edge limit is inclusive");
    assert!(matches!(
        validate_size(MAX_BISECTION_EDGES + 1),
        Err(Error::TooLarge(_))
    ));
}

#[test]
fn build_csr_sorts_each_row_and_collapses_repeats_and_self_loops() {
    // Vertex 3 is isolated, (0, 1) appears twice in both directions, and
    // (2, 2) is a self-loop: none of the three may reach the adjacency.
    let graph = build_csr(4, &[(1, 0), (0, 1), (2, 2), (0, 2), (1, 0)]);
    assert_eq!(graph.num_vertices(), 4);
    assert_eq!(graph.neighbors(0), &[1, 2]);
    assert_eq!(graph.neighbors(1), &[0]);
    assert_eq!(graph.neighbors(2), &[0]);
    assert!(graph.neighbors(3).is_empty());
    assert_eq!(graph.edge_weights.len(), graph.neighbors.len());
    assert!(graph.edge_weights.iter().all(|&w| w == 1));
    assert_eq!(graph.vertex_weights, vec![1; 4]);
}

#[test]
fn build_csr_on_a_graph_with_no_edges_gives_every_vertex_an_empty_row() {
    let graph = build_csr(3, &[]);
    assert_eq!(graph.offsets, vec![0, 0, 0, 0]);
    assert!(graph.neighbors.is_empty());
}
