use goatd::{
    Graph, TreeDecomposition,
    elimination::{self, Order, Prepared, RunConfig, RunOutcome},
};
use std::time::Duration;

fn graph() -> Graph {
    Graph::new(
        25,
        (0..25).flat_map(|v| {
            [
                (v % 5 < 4).then_some((v, v + 1)),
                (v / 5 < 4).then_some((v, v + 5)),
            ]
            .into_iter()
            .flatten()
        }),
    )
}

fn completed(outcome: RunOutcome) -> TreeDecomposition {
    match outcome {
        RunOutcome::Completed(tree) | RunOutcome::CompletedAtDeadline(_, tree) => tree,
        _ => panic!("expected a complete candidate"),
    }
}

#[test]
fn prepared_orders_match_independent_runs_after_weights_and_seeds_change() {
    let graph = graph();
    let mut prepared = Prepared::new(&graph, None).unwrap();
    let weights: Vec<_> = (0..25).map(|v| v * 17).collect();
    for order in [
        Order::MinDegree,
        Order::MinFillSampled { weights: &weights },
        Order::NestedDissection,
        Order::MinDegreeSampled { weights: &[0; 25] },
    ] {
        for seed in [2, 7] {
            let reused = completed(prepared.run(order, seed, RunConfig::default()).unwrap());
            reused.validate(&graph).unwrap();
            assert_eq!(
                reused.to_td(),
                elimination::decompose(&graph, order, seed, None)
                    .unwrap()
                    .to_td()
            );
        }
    }
    assert_eq!(prepared.preparation().original_vertices, 25);
    assert!(prepared.preparation().residual_vertices < 25);
}

#[test]
fn failed_and_pruned_runs_do_not_poison_later_candidates() {
    let graph = graph();
    let mut prepared = Prepared::new(&graph, None).unwrap();
    assert!(
        prepared
            .run(
                Order::MinFillSampled { weights: &[0] },
                1,
                RunConfig::default()
            )
            .is_err()
    );
    assert!(
        prepared
            .run(
                Order::MinFill,
                1,
                RunConfig::default().with_budgets(None, Some(Duration::ZERO))
            )
            .is_err()
    );
    assert!(matches!(
        prepared
            .run(
                Order::MinFill,
                1,
                RunConfig::default().with_width_bound(Some(0))
            )
            .unwrap(),
        RunOutcome::WidthAborted
    ));
    let budget = RunConfig::default().with_budgets(Some(Duration::ZERO), None);
    assert!(matches!(
        prepared
            .run(
                Order::MinFillSampled { weights: &[0; 25] },
                1,
                budget.with_deadline_completion(false)
            )
            .unwrap(),
        RunOutcome::DeadlineAborted(_)
    ));
    completed(prepared.run(Order::MinDegree, 1, budget).unwrap())
        .validate(&graph)
        .unwrap();
    let tree = completed(
        prepared
            .run(Order::MinFill, 1, RunConfig::default())
            .unwrap(),
    );
    assert_eq!(
        tree.to_td(),
        elimination::decompose(&graph, Order::MinFill, 1, None)
            .unwrap()
            .to_td()
    );
}

#[test]
fn prepared_bound_includes_reduced_prefix() {
    use goatd::{
        Graph,
        elimination::{Order, Prepared, RunConfig, RunOutcome},
    };
    let graph = Graph::new(3, [(0, 1), (1, 2), (0, 2)]);
    let mut prepared = Prepared::new(&graph, None).unwrap();
    assert_eq!(prepared.preparation().residual_vertices, 0);
    assert!(matches!(
        prepared
            .run(
                Order::MinFill,
                0,
                RunConfig::default().with_width_bound(Some(1))
            )
            .unwrap(),
        RunOutcome::WidthAborted
    ));
}

#[test]
fn reduced_prefix_accepts_an_equal_bound_after_an_abort() {
    let graph = Graph::new(3, [(0, 1), (1, 2), (0, 2)]);
    let mut prepared = Prepared::new(&graph, None).unwrap();
    assert!(matches!(
        prepared
            .run(
                Order::MinFill,
                0,
                RunConfig::default().with_width_bound(Some(1))
            )
            .unwrap(),
        RunOutcome::WidthAborted
    ));
    let tree = completed(
        prepared
            .run(
                Order::MinFill,
                0,
                RunConfig::default().with_width_bound(Some(2)),
            )
            .unwrap(),
    );
    tree.validate(&graph).unwrap();
    assert_eq!(tree.treewidth(), 2);
}

#[test]
fn deadline_completion_obeys_the_width_bound() {
    let graph = graph();
    let mut prepared = Prepared::new(&graph, None).unwrap();
    let config = RunConfig::default().with_budgets(Some(Duration::ZERO), None);
    let tree = completed(prepared.run(Order::MinFill, 0, config).unwrap());
    tree.validate(&graph).unwrap();
    let width = tree.treewidth();
    assert!(width > 0);
    assert!(matches!(
        prepared
            .run(Order::MinFill, 0, config.with_width_bound(Some(width - 1)))
            .unwrap(),
        RunOutcome::WidthAborted
    ));
    let tree = completed(
        prepared
            .run(Order::MinFill, 0, config.with_width_bound(Some(width)))
            .unwrap(),
    );
    tree.validate(&graph).unwrap();
}

#[test]
fn a_reduced_prefix_is_checked_even_when_a_residual_run_cannot_start() {
    for joined in [false, true] {
        let mut edges = vec![(0, 1), (1, 2), (0, 2)];
        edges.extend((0..8u32).flat_map(|v| {
            [1, 2, 4].into_iter().filter_map(move |bit| {
                let w = v ^ bit;
                (v < w).then_some((v + 3, w + 3))
            })
        }));
        if joined {
            edges.push((2, 3));
        }
        let graph = Graph::new(11, edges);
        let mut prepared = Prepared::new(&graph, None).unwrap();
        assert!(prepared.preparation().residual_vertices > 0);
        assert!(prepared.preparation().residual_vertices < 11);
        let config = RunConfig::default()
            .with_width_bound(Some(1))
            .with_budgets(Some(Duration::ZERO), None)
            .with_deadline_completion(false);
        assert_eq!(
            prepared.run(Order::MinFill, 0, config).unwrap(),
            RunOutcome::WidthAborted
        );
        completed(
            prepared
                .run(Order::MinFill, 0, RunConfig::default())
                .unwrap(),
        )
        .validate(&graph)
        .unwrap();
    }
}
