use std::time::Duration;

use goatd::Graph;
use goatd::portfolio::{
    PortfolioConfig, candidates, decompose, decompose_and_refine, sampled_min_fill_candidates,
};

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
fn every_portfolio_entry_point_decomposes_a_grid() {
    let graph = grid(4);
    let weight = vec![1; graph.num_vertices() as usize];

    let candidates = candidates(&graph, &weight, 0, PortfolioConfig::standard()).unwrap();
    assert!(!candidates.is_empty());
    for decomposition in &candidates {
        decomposition.validate(&graph).unwrap();
    }
    let quality: Vec<(u32, usize)> = candidates
        .iter()
        .map(|td| (td.treewidth(), td.total_bag_size()))
        .collect();
    assert!(quality.windows(2).all(|pair| pair[0] <= pair[1]));

    let refined =
        decompose_and_refine(&graph, &weight, 0, PortfolioConfig::standard(), None).unwrap();
    refined.validate(&graph).unwrap();
    assert!((refined.treewidth(), refined.total_bag_size()) <= quality[0]);

    let sampled =
        sampled_min_fill_candidates(&graph, &weight, 0, PortfolioConfig::sampled_min_fill())
            .unwrap();
    assert!(!sampled.is_empty());
    for decomposition in &sampled {
        decomposition.validate(&graph).unwrap();
    }
}

#[test]
fn portfolio_configuration_rejects_invalid_durations() {
    let graph = grid(2);
    let weight = vec![1; graph.num_vertices() as usize];
    let invalid = [
        PortfolioConfig::standard().with_flowcutter(Duration::from_millis(49)),
        PortfolioConfig::standard().with_flowcutter(Duration::MAX),
        PortfolioConfig::standard().with_soft_budget(Duration::MAX),
    ];

    for config in invalid {
        assert!(candidates(&graph, &weight, 0, config).is_err());
    }
}

#[test]
fn zero_extra_sampling_runs_still_runs_the_initial_candidates() {
    let graph = grid(2);
    let weight = vec![1; graph.num_vertices() as usize];
    let candidates = candidates(
        &graph,
        &weight,
        0,
        PortfolioConfig::standard().with_sampling_runs(0),
    )
    .unwrap();

    assert!(!candidates.is_empty());
    assert!(
        candidates
            .iter()
            .all(|decomposition| decomposition.validate(&graph).is_ok())
    );
}

#[test]
fn the_portfolio_winner_has_no_bag_subsumed_by_a_neighbour() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3)]);
    let weight = vec![1; graph.num_vertices() as usize];
    let decomposition = decompose(
        &graph,
        &weight,
        0,
        PortfolioConfig::standard().with_sampling_runs(0),
    )
    .unwrap();

    for (bag, neighbours) in decomposition.adjacency().iter().enumerate() {
        for &neighbour in neighbours {
            let left = decomposition.bags()[bag].vertices();
            let right = decomposition.bags()[neighbour].vertices();
            assert!(
                !left.iter().all(|vertex| right.contains(vertex)),
                "bag {bag} is subsumed by adjacent bag {neighbour}"
            );
        }
    }
}
