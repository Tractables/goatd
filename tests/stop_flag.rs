//! The stop flag ends a running solve and leaves the caller a decomposition.
//!
//! The flag is process-wide, so this file holds exactly one test: a second one
//! running beside it would see a flag it did not set.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use goatd::partition::{
    GraphBisectionConfig, Hypergraph, HypergraphBisectionConfig, multilevel_graph_bisect,
    multilevel_hypergraph_bisect,
};
use goatd::portfolio::{PortfolioConfig, decompose};
use goatd::{Graph, stop_flag};

/// A grid large enough that a ten-second portfolio would keep going for
/// seconds, so a run that returns at once returned because it was asked to.
fn grid(side: u32) -> Graph {
    let mut edges = Vec::new();
    for row in 0..side {
        for col in 0..side {
            let vertex = row * side + col;
            if col + 1 < side {
                edges.push((vertex, vertex + 1));
            }
            if row + 1 < side {
                edges.push((vertex, vertex + side));
            }
        }
    }
    Graph::new(side * side, edges)
}

/// The same grid as a hypergraph, one two-pin hyperedge per edge.
fn grid_hypergraph(side: u32) -> Hypergraph {
    let mut hyperedges = Vec::new();
    for row in 0..side {
        for col in 0..side {
            let vertex = row * side + col;
            if col + 1 < side {
                hyperedges.push(vec![vertex, vertex + 1]);
            }
            if row + 1 < side {
                hyperedges.push(vec![vertex, vertex + side]);
            }
        }
    }
    Hypergraph::new(side * side, &hyperedges, None).expect("a grid is a hypergraph")
}

#[test]
fn a_set_stop_flag_returns_the_decomposition_found_so_far() {
    let graph = grid(60);
    let config = PortfolioConfig::standard_with_budget(Duration::from_secs(10));
    let weights = vec![1; graph.num_vertices() as usize];

    stop_flag().store(true, Ordering::Relaxed);
    let started = Instant::now();
    let result = decompose(&graph, &weights, 0, config);
    let elapsed = started.elapsed();
    // The bisection reads the flag too, and the public entry point answers a
    // stopped bisection with the index split rather than a panic.
    let bisection = multilevel_graph_bisect(&graph, GraphBisectionConfig::new(0.1, 7));
    let hypergraph_bisection =
        multilevel_hypergraph_bisect(&grid_hypergraph(60), HypergraphBisectionConfig::new(0.1, 7));
    stop_flag().store(false, Ordering::Relaxed);

    for (what, bisection) in [("graph", bisection), ("hypergraph", hypergraph_bisection)] {
        let bisection = bisection.expect("a stopped bisection is not an error");
        assert!(
            bisection.parts().contains(&0) && bisection.parts().contains(&1),
            "a stopped {what} bisection still names both sides",
        );
    }

    let td = result.expect("the portfolio still returns a decomposition");
    td.validate(&graph)
        .expect("the decomposition it returns is valid for the graph");
    assert!(
        elapsed < Duration::from_secs(5),
        "a stopped run returns well inside its budget, took {elapsed:?}",
    );
}
