use std::time::Duration;

use goatd::Graph;
use goatd::portfolio::{
    Hedge, Pass, PortfolioConfig, Stage, candidates, decompose, decompose_and_refine,
    decompose_traced, sampled_min_fill_candidates,
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
        PortfolioConfig::standard().with_hard_budget(Duration::from_secs(1)),
        PortfolioConfig::standard()
            .with_soft_budget(Duration::from_secs(2))
            .with_hard_budget(Duration::from_secs(1)),
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

/// The restarts of a run, and how many candidates it ran on the hedge's
/// ranking.
fn restarts(graph: &Graph, config: PortfolioConfig) -> (Vec<(u64, Pass)>, usize) {
    let weight = vec![1; graph.num_vertices() as usize];
    let mut samples = Vec::new();
    let mut modified = 0;
    decompose_traced(graph, &weight, 0, config, &mut |candidate| {
        if matches!(candidate.pass, Pass::Modified { .. }) {
            modified += 1;
        }
        if candidate.stage == Stage::Sample {
            samples.push((candidate.seed, candidate.pass));
        }
    })
    .expect("a decomposition");
    (samples, modified)
}

#[test]
fn the_default_portfolio_hedges_and_leaves_the_restarts_alone() {
    let graph = grid(6);

    let (hedged, hedged_twice) = restarts(&graph, PortfolioConfig::standard());
    let (plain, plain_twice) = restarts(&graph, PortfolioConfig::standard().with_hedge(Hedge::Off));

    assert!(hedged_twice > 0, "the default runs a modified pass");
    assert_eq!(plain_twice, 0, "Hedge::Off runs each candidate once");
    assert_eq!(hedged.len(), plain.len(), "the restart count is the same");
    for (index, (left, right)) in hedged.iter().zip(&plain).enumerate() {
        assert_eq!(left.0, right.0, "restart {index} runs another seed");
        assert_eq!(left.1, Pass::Plain, "restart {index} is not plain");
        assert_eq!(right.1, Pass::Only, "restart {index} of an unhedged run");
    }
}

#[test]
fn a_budgeted_portfolio_draws_restart_seeds_until_the_soft_deadline() {
    let graph = grid(6);
    let to_deadline = PortfolioConfig::standard_with_budget(Duration::from_millis(300))
        .with_hedge(Hedge::Off)
        .with_sampling_runs(3);
    let capped = to_deadline.with_restarts_to_deadline(false);

    let (capped_runs, _) = restarts(&graph, capped);
    let (extended, _) = restarts(&graph, to_deadline);

    assert_eq!(capped_runs.len(), 3, "capped, the count stops the restarts");
    assert!(
        extended.len() > capped_runs.len(),
        "the deadline ran {} restarts, the count {}",
        extended.len(),
        capped_runs.len()
    );
    // The extra restarts carry on from the next seed of the same sequence.
    for (index, (left, right)) in capped_runs.iter().zip(&extended).enumerate() {
        assert_eq!(left.0, right.0, "restart {index} runs another seed");
    }
}

#[test]
fn the_restarts_stop_at_their_count_without_a_deadline_to_run_to() {
    let graph = grid(6);
    let config = PortfolioConfig::standard()
        .with_hedge(Hedge::Off)
        .with_sampling_runs(3)
        .with_restarts_to_deadline(true);

    let (samples, _) = restarts(&graph, config);

    assert_eq!(samples.len(), 3, "no deadline, so the count stops them");
}

#[test]
fn a_soft_budget_on_the_standard_portfolio_keeps_the_restart_count() {
    let graph = grid(6);
    let config = PortfolioConfig::standard()
        .with_hedge(Hedge::Off)
        .with_soft_budget(Duration::from_millis(300))
        .with_sampling_runs(3);

    let (samples, _) = restarts(&graph, config);

    assert_eq!(
        samples.len(),
        3,
        "standard() leaves the restarts at their count under a deadline"
    );
}

/// Every candidate a run traced, in order.
fn traced_stages(graph: &Graph, config: PortfolioConfig) -> Vec<(Stage, Pass)> {
    let weight = vec![1; graph.num_vertices() as usize];
    let mut seen = Vec::new();
    decompose_traced(graph, &weight, 0, config, &mut |candidate| {
        seen.push((candidate.stage, candidate.pass));
    })
    .expect("a decomposition");
    seen
}

#[test]
fn the_expensive_orders_stop_at_the_configured_residual() {
    let graph = grid(6);
    let config = PortfolioConfig::standard();

    let ordinary = traced_stages(&graph, config);
    // A limit of zero puts every graph above the line, whatever its size.
    let min_degree_only = traced_stages(&graph, config.with_expensive_orders_up_to(0));
    // The grid is far below the default limit, so naming it changes nothing.
    let at_the_default = traced_stages(&graph, config.with_expensive_orders_up_to(300_000));

    assert!(
        ordinary.iter().any(|&(stage, _)| stage == Stage::MinFill),
        "a small graph runs the expensive orders: {ordinary:?}"
    );
    assert!(
        ordinary.iter().any(|&(stage, _)| stage == Stage::Sample),
        "its restarts are sampled min-fill: {ordinary:?}"
    );
    assert!(
        ordinary
            .iter()
            .any(|&(_, pass)| matches!(pass, Pass::Modified { .. })),
        "and the hedge runs: {ordinary:?}"
    );
    // What the limit governs is the schedule's own choice of candidates. The
    // ones carrying a vertex cap of their own answer that cap instead, so they
    // are still traced here; the grid is far below every one of those caps.
    assert!(
        min_degree_only.iter().all(|&(stage, pass)| {
            matches!(
                stage,
                Stage::MinDegree
                    | Stage::MaximumCardinality
                    | Stage::MinimalTriangulation
                    | Stage::Minimalized
                    | Stage::FlowCutter
            ) && pass == Pass::Only
        }),
        "above the limit min-degree runs, plus the self-gated candidates: {min_degree_only:?}"
    );
    assert!(
        min_degree_only
            .iter()
            .any(|&(stage, _)| stage == Stage::MinDegree),
        "and min-degree is among them: {min_degree_only:?}"
    );
    assert_eq!(at_the_default, ordinary, "300,000 is the default limit");
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

#[test]
fn the_minimal_triangulation_candidate_can_be_gated_off_and_on() {
    let graph = grid(5);
    let weight = vec![1; graph.num_vertices() as usize];
    let mut stages = Vec::new();
    decompose_traced(
        &graph,
        &weight,
        0,
        PortfolioConfig::standard(),
        &mut |candidate| stages.push(candidate.stage),
    )
    .unwrap();
    assert!(
        stages.contains(&Stage::MinimalTriangulation),
        "the standard portfolio runs the candidate on a graph this size"
    );

    stages.clear();
    decompose_traced(
        &graph,
        &weight,
        0,
        PortfolioConfig::standard().without_minimal_triangulation(),
        &mut |candidate| stages.push(candidate.stage),
    )
    .unwrap();
    assert!(!stages.contains(&Stage::MinimalTriangulation));
}

#[test]
fn dropping_fill_never_widens_the_portfolio_winner() {
    let graph = grid(6);
    let weight = vec![1; graph.num_vertices() as usize];
    let kept = decompose(
        &graph,
        &weight,
        0,
        PortfolioConfig::standard().without_triangulation_refinement(),
    )
    .unwrap();
    let dropped = decompose(&graph, &weight, 0, PortfolioConfig::standard()).unwrap();
    dropped.validate(&graph).unwrap();
    assert!(dropped.treewidth() <= kept.treewidth());
}

#[test]
fn the_maximum_cardinality_candidate_can_be_gated_off_and_on() {
    let graph = grid(5);
    let weight = vec![1; graph.num_vertices() as usize];
    let mut stages = Vec::new();
    decompose_traced(
        &graph,
        &weight,
        0,
        PortfolioConfig::standard(),
        &mut |candidate| stages.push(candidate.stage),
    )
    .unwrap();
    assert!(
        stages.contains(&Stage::MaximumCardinality),
        "the standard portfolio runs the candidate on a graph this size"
    );
    let plain = stages
        .iter()
        .position(|stage| *stage == Stage::MaximumCardinality);
    let paths = stages
        .iter()
        .position(|stage| *stage == Stage::MinimalTriangulation);
    assert!(plain < paths, "the cheaper search runs first: {stages:?}");

    stages.clear();
    decompose_traced(
        &graph,
        &weight,
        0,
        PortfolioConfig::standard().without_maximum_cardinality(),
        &mut |candidate| stages.push(candidate.stage),
    )
    .unwrap();
    assert!(!stages.contains(&Stage::MaximumCardinality));
    assert!(stages.contains(&Stage::MinimalTriangulation));
}

#[test]
fn a_gate_below_the_residual_leaves_the_maximum_cardinality_candidate_unrun() {
    let graph = grid(5);
    let weight = vec![1; graph.num_vertices() as usize];
    let mut stages = Vec::new();
    decompose_traced(
        &graph,
        &weight,
        0,
        PortfolioConfig::standard().with_maximum_cardinality(0),
        &mut |candidate| stages.push(candidate.stage),
    )
    .unwrap();
    assert!(!stages.contains(&Stage::MaximumCardinality));
}
