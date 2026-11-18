//! A wall cutoff bounds a metered solve without changing one it does not reach.

use std::time::{Duration, Instant};

use goatd::Graph;
use goatd::deadline::{WallGuard, wall_cutoff};
use goatd::meter::arm;
use goatd::portfolio::{PortfolioConfig, decompose};

/// A grid large enough that a ten-second portfolio keeps going for seconds.
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

#[test]
fn nested_cutoffs_keep_the_earliest_and_restore_on_drop() {
    let soon = Instant::now() + Duration::from_secs(60);
    let later = soon + Duration::from_secs(60);

    assert_eq!(wall_cutoff(), None);
    {
        let _outer = WallGuard::new(Some(soon));
        {
            let _inner = WallGuard::new(Some(later));
            assert_eq!(wall_cutoff(), Some(soon));
            let _none = WallGuard::new(None);
            assert_eq!(wall_cutoff(), Some(soon));
        }
        assert_eq!(wall_cutoff(), Some(soon));
    }
    assert_eq!(wall_cutoff(), None);
}

#[test]
fn a_passed_cutoff_ends_a_metered_portfolio_with_a_decomposition() {
    let graph = grid(60);
    let weights = vec![1; graph.num_vertices() as usize];
    let config = PortfolioConfig::standard_with_budget(Duration::from_secs(10));

    let started = Instant::now();
    let result = {
        let _meter = arm(Instant::now());
        let _cutoff = WallGuard::new(Some(Instant::now()));
        decompose(&graph, &weights, 0, config)
    };
    let elapsed = started.elapsed();

    result
        .expect("the portfolio still returns a decomposition")
        .validate(&graph)
        .expect("the decomposition it returns is valid for the graph");
    assert!(
        elapsed < Duration::from_secs(5),
        "a run past its cutoff returns well inside its budget, took {elapsed:?}",
    );
}
