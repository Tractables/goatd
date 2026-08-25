//! The step-budgeted search on the smallest graphs there are.
//!
//! A step budget is what makes a decomposition reproducible — the wall-clock
//! mode stops wherever the clock happens to fall — so these shapes are asked
//! for twice.

use crate::Graph;
use crate::flowcutter::{FcBudget, flowcutter_td};
use crate::tests::td_fixture::{GraphAtItsWidth, assert_valid_td};

/// Six shapes at their own treewidths. No decomposition can go below a graph's
/// treewidth, so an upper bound pins the width exactly.
#[test]
fn a_step_budgeted_decomposition_of_the_tiny_shapes_is_valid_and_the_same_tree_twice() {
    let shapes: &[GraphAtItsWidth] = &[
        ("one edge", 2, &[(0, 1)], 1),
        ("a path", 5, &[(0, 1), (1, 2), (2, 3), (3, 4)], 1),
        ("a cycle", 4, &[(0, 1), (1, 2), (2, 3), (3, 0)], 2),
        ("a star", 4, &[(0, 1), (0, 2), (0, 3)], 1),
        (
            "a complete graph",
            4,
            &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)],
            3,
        ),
        (
            "two triangles",
            6,
            &[(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)],
            2,
        ),
    ];

    let budget = FcBudget::Steps {
        steps: 10_000,
        iters: 8,
    };
    for &(shape, num_vertices, edges, treewidth) in shapes {
        let graph = Graph::new(num_vertices, edges.iter().copied());
        let td =
            flowcutter_td(&graph, budget).unwrap_or_else(|e| panic!("{shape} must decompose: {e}"));
        assert_valid_td(&td, num_vertices, edges);
        assert!(
            td.treewidth() <= treewidth,
            "{shape} must reach width {treewidth}, got {}",
            td.treewidth(),
        );

        let again =
            flowcutter_td(&graph, budget).unwrap_or_else(|e| panic!("{shape} must decompose: {e}"));
        assert_eq!(
            td.bags, again.bags,
            "{shape} must decompose the same way under the same step budget",
        );
        assert_eq!(td.adj, again.adj, "{shape} must give the same bag tree");
    }
}
