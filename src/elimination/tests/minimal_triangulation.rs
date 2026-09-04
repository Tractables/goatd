//! MCS-M on graphs whose treewidth is known, and the ordering's determinism.

use crate::elimination::{Order, decompose};
use crate::{Graph, TreeDecomposition};

/// A cycle on `n` vertices: treewidth 2.
fn cycle(n: u32) -> Graph {
    Graph::new(n, (0..n).map(|vertex| (vertex, (vertex + 1) % n)))
}

/// The `rows` by `columns` grid: treewidth `min(rows, columns)`.
fn grid(rows: u32, columns: u32) -> Graph {
    let mut edges = Vec::new();
    for row in 0..rows {
        for column in 0..columns {
            let vertex = row * columns + column;
            if column + 1 < columns {
                edges.push((vertex, vertex + 1));
            }
            if row + 1 < rows {
                edges.push((vertex, vertex + columns));
            }
        }
    }
    Graph::new(rows * columns, edges)
}

/// The complete bipartite graph on `left` and `right` vertices: treewidth
/// `min(left, right)`.
fn complete_bipartite(left: u32, right: u32) -> Graph {
    let edges = (0..left).flat_map(|a| (0..right).map(move |b| (a, left + b)));
    Graph::new(left + right, edges)
}

fn minimal_triangulation(graph: &Graph) -> TreeDecomposition {
    let td = decompose(graph, Order::MinimalTriangulation, 0, None)
        .expect("a deterministic order takes no weights");
    td.validate(graph)
        .expect("the order produces a decomposition of its graph");
    td
}

#[test]
fn cycle_is_triangulated_to_width_two() {
    // Every minimal triangulation of a cycle is a triangulation of the polygon
    // it bounds, whose maximal cliques are triangles.
    for n in [4, 7, 10, 25] {
        let graph = cycle(n);
        assert_eq!(minimal_triangulation(&graph).treewidth(), 2, "cycle of {n}");
    }
}

#[test]
fn grids_are_triangulated_near_their_treewidth() {
    // A minimal triangulation is one no fill edge can be dropped from, which is
    // not the same as a narrowest one, so the search can land a step above the
    // treewidth. The third case is such a graph. The widths are fixed because
    // the search reads no seed.
    for (rows, columns, expected) in [(3, 3, 3), (4, 4, 4), (3, 6, 4)] {
        let graph = grid(rows, columns);
        let width = minimal_triangulation(&graph).treewidth();
        assert!(
            width >= rows.min(columns),
            "the {rows}x{columns} grid has treewidth {}, and no decomposition is narrower",
            rows.min(columns)
        );
        assert_eq!(width, expected, "the {rows}x{columns} grid");
    }
}

#[test]
fn complete_bipartite_graphs_are_triangulated_near_their_treewidth() {
    for (left, right, expected) in [(2, 5, 2), (3, 3, 3), (3, 4, 4)] {
        let graph = complete_bipartite(left, right);
        let width = minimal_triangulation(&graph).treewidth();
        assert!(
            width >= left.min(right),
            "K_{left},{right} has treewidth {}, and no decomposition is narrower",
            left.min(right)
        );
        assert_eq!(width, expected, "K_{left},{right}");
    }
}

#[test]
fn a_complete_graph_needs_no_fill() {
    let n = 6;
    let edges = (0..n).flat_map(|a| (a + 1..n).map(move |b| (a, b)));
    let graph = Graph::new(n, edges);
    assert_eq!(minimal_triangulation(&graph).treewidth(), n - 1);
}

#[test]
fn the_order_repeats() {
    let graph = grid(5, 6);
    let first = minimal_triangulation(&graph);
    for seed in [0, 1, 12_345] {
        let again = decompose(&graph, Order::MinimalTriangulation, seed, None)
            .expect("a deterministic order takes no weights");
        assert_eq!(again, first, "MCS-M reads no seed, so every run agrees");
    }
}
