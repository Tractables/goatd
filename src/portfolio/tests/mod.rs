use std::cell::OnceCell;
use std::time::{Duration, Instant};

use super::candidates::CandidateSet;
use super::config::{MAX_DIVERSE_SAMPLING_RUNS, validate};
use super::{CandidateOutcome, EliminationPhase, Hedge, ModifiedWeights, Pass, PortfolioConfig};
use super::{Sample, Schedule, Stage, elimination_stop, extra_sample, sample_seed};
use crate::elimination::Order;
use crate::{Graph, TreeDecomposition};

/// An unhedged schedule: the caller's weights on every candidate, one diverse
/// pass, no second one to run against it.
fn schedule(base_seed: u64, large_residual: bool, weights: &[u32]) -> Schedule<'_> {
    Schedule {
        base_seed,
        large_residual,
        ordinary_runs: 1_000,
        diverse_runs: 46,
        modified: None,
        fixed_runs: 0,
        initial_orders: super::standard_orders,
        weights,
    }
}

/// A hedged schedule whose ranking is already in `cell`, so the candidates can
/// be read without placing one.
fn hedged<'a>(
    base_seed: u64,
    graph: &'a Graph,
    cell: &'a OnceCell<Vec<u32>>,
    weights: &'a [u32],
    fixed_runs: u64,
) -> Schedule<'a> {
    Schedule {
        base_seed,
        large_residual: false,
        ordinary_runs: 1_000,
        diverse_runs: 46,
        modified: Some(ModifiedWeights {
            cell,
            graph,
            dim: 3,
            rounds: 1_000,
            seed: base_seed,
            soft_deadline: None,
        }),
        fixed_runs,
        initial_orders: super::standard_orders,
        weights,
    }
}

#[test]
fn best_only_candidate_storage_discards_losing_decompositions() {
    let graph = Graph::new(3, []);
    let wide = TreeDecomposition::new(&graph, [vec![0, 1, 2]], []).unwrap();
    let narrow =
        TreeDecomposition::new(&graph, [vec![0], vec![1], vec![2]], [(0, 1), (1, 2)]).unwrap();
    let medium = TreeDecomposition::new(&graph, [vec![0, 1], vec![1, 2]], [(0, 1)]).unwrap();
    let mut candidates = CandidateSet::best_only();

    candidates.push(wide);
    candidates.push(narrow);
    candidates.push(medium);

    let retained = candidates.into_decompositions();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].treewidth(), 0);
}

#[test]
fn best_only_candidate_storage_compares_compacted_bag_size() {
    let graph = Graph::new(3, []);
    let smaller_before_compaction =
        TreeDecomposition::new(&graph, [vec![0, 1], vec![1, 2]], [(0, 1)]).unwrap();
    let smaller_after_compaction = TreeDecomposition::new(
        &graph,
        [vec![0, 1], vec![0], vec![0], vec![2]],
        [(0, 1), (1, 2)],
    )
    .unwrap();
    let mut candidates = CandidateSet::best_only();

    candidates.push(smaller_before_compaction);
    candidates.push(smaller_after_compaction);

    let retained = candidates.into_decompositions();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].quality_key(), (1, 3));
}

#[test]
fn a_short_standard_budget_keeps_the_fast_sampling_cap() {
    let budget = Duration::from_millis(4_749);
    let config = PortfolioConfig::standard_with_budget(budget);

    assert_eq!(config.soft_budget, Some(budget));
    assert_eq!(config.sampling_runs, 100);
    assert_eq!(config.diverse_sampling_runs, 0);
    assert_eq!(config.flowcutter_budget, None);
}

#[test]
fn a_ten_second_outer_window_with_output_headroom_raises_the_sampling_cap() {
    let budget = Duration::from_millis(4_750);
    let config = PortfolioConfig::standard_with_budget(budget);

    assert_eq!(config.soft_budget, Some(budget));
    assert_eq!(config.sampling_runs, 1_000);
    assert_eq!(config.diverse_sampling_runs, 46);
    assert_eq!(config.flowcutter_budget, Some(budget));
}

#[test]
fn the_standard_portfolio_starts_with_update_order_minimum_degree() {
    let weights = [1; 3];
    let candidates = super::standard_orders(0, &weights);

    assert_eq!(candidates.len(), 6);
    let candidate = candidates[0];
    assert_eq!(candidate.order, Order::MinDegree);
    assert!(!candidate.preprocess);
    assert!(candidate.update_order_ties);
    assert!(
        candidates[1..]
            .iter()
            .all(|candidate| candidate.preprocess && !candidate.update_order_ties)
    );
}

#[test]
fn diverse_orders_can_be_taken_without_the_flowcutter_slot() {
    let budget = Duration::from_millis(4_750);
    let config = PortfolioConfig::standard()
        .with_soft_budget(budget)
        .with_sampling_runs(1_000)
        .with_diverse_sampling_runs(MAX_DIVERSE_SAMPLING_RUNS);

    assert_eq!(config.sampling_runs, 1_000);
    assert_eq!(config.diverse_sampling_runs, MAX_DIVERSE_SAMPLING_RUNS);
    assert_eq!(config.flowcutter_budget, None);
    assert!(validate(config).is_ok());
}

#[test]
fn more_diverse_orders_than_the_sampler_has_is_rejected() {
    let config =
        PortfolioConfig::standard().with_diverse_sampling_runs(MAX_DIVERSE_SAMPLING_RUNS + 1);

    let message = validate(config).unwrap_err().to_string();
    assert!(
        message.contains("diverse sampling runs"),
        "unexpected message: {message}"
    );
}

#[test]
fn an_explicit_hard_budget_does_not_change_the_soft_schedule() {
    let soft = Duration::from_millis(4_750);
    let hard = Duration::from_millis(9_000);
    let config = PortfolioConfig::standard_with_budget(soft).with_hard_budget(hard);

    assert_eq!(config.soft_budget, Some(soft));
    assert_eq!(config.hard_budget, Some(hard));
    assert_eq!(config.sampling_runs, 1_000);
    assert_eq!(config.diverse_sampling_runs, 46);
    assert_eq!(config.flowcutter_budget, Some(soft));
}

#[test]
fn every_standard_portfolio_hedges_by_default() {
    let hedge = Hedge::EccentricityPasses {
        dim: 3,
        rounds: 1_000,
    };

    assert_eq!(Hedge::eccentricity(), hedge);
    assert_eq!(PortfolioConfig::standard().hedge, hedge);
    assert_eq!(
        PortfolioConfig::standard_with_budget(Duration::from_millis(4_750)).hedge,
        hedge
    );
    assert_eq!(
        PortfolioConfig::standard_with_budget(Duration::from_millis(500)).hedge,
        hedge
    );
    assert_eq!(PortfolioConfig::sampled_min_fill().hedge, Hedge::Off);
    assert_eq!(
        PortfolioConfig::standard().with_hedge(Hedge::Off).hedge,
        Hedge::Off,
        "a caller can turn it off"
    );
}

#[test]
fn a_hedge_placement_that_could_never_run_is_rejected() {
    let no_dimensions =
        PortfolioConfig::standard().with_hedge(Hedge::EccentricityPasses { dim: 0, rounds: 8 });
    let too_many_dimensions =
        PortfolioConfig::standard().with_hedge(Hedge::EccentricityPasses { dim: 9, rounds: 8 });
    let no_rounds =
        PortfolioConfig::standard().with_hedge(Hedge::EccentricityPasses { dim: 2, rounds: 0 });

    assert!(super::config::validate(no_dimensions).is_err());
    assert!(super::config::validate(too_many_dimensions).is_err());
    assert!(super::config::validate(no_rounds).is_err());
    assert!(super::config::validate(PortfolioConfig::standard()).is_ok());
}

#[test]
fn diverse_samples_precede_the_complete_ordinary_schedule() {
    let weights = [1; 3];
    let plan = schedule(0, false, &weights);
    let large = schedule(0, true, &weights);
    let expected_coefficients = [1, -1, -2, -3, -4, -5, -8, -7, -16, -32];

    for (index, expected) in expected_coefficients.into_iter().enumerate() {
        let sample = extra_sample(plan, index as u64).unwrap();
        let Order::FillDegreeSampled {
            degree_coefficient, ..
        } = sample.order
        else {
            panic!("diverse sample {index} did not use a fill-degree order");
        };
        assert_eq!(degree_coefficient, expected);
        assert_eq!(sample.pass, Pass::Only, "nothing is hedged here");
    }
    assert!(matches!(
        extra_sample(plan, 46),
        Some(Sample {
            order: Order::MinFillSampled { .. },
            ..
        })
    ));
    assert!(matches!(
        extra_sample(plan, 1_045),
        Some(Sample {
            order: Order::MinFillSampled { .. },
            ..
        })
    ));
    assert!(extra_sample(plan, 1_046).is_none());
    assert!(matches!(
        extra_sample(large, 999),
        Some(Sample {
            order: Order::MinDegreeSampled { .. },
            ..
        })
    ));
    assert!(extra_sample(large, 1_000).is_none());
}

#[test]
fn diverse_samples_do_not_shift_the_ordinary_seed_stream() {
    let base_seed = 17;
    let weights = [1; 3];
    let plan = schedule(base_seed, false, &weights);
    let replay_coefficients = [-3, -5, -8, -16];

    for index in 0..10 {
        let sample = extra_sample(plan, index).unwrap();
        assert_eq!(sample.seed, sample_seed(base_seed, 0));
    }
    for replay_seed_index in 1u64..=9 {
        for (coefficient_index, expected_coefficient) in replay_coefficients.into_iter().enumerate()
        {
            let index = 10 + (replay_seed_index - 1) * 4 + coefficient_index as u64;
            let sample = extra_sample(plan, index).unwrap();
            let Order::FillDegreeSampled {
                degree_coefficient, ..
            } = sample.order
            else {
                panic!("replayed diverse sample did not use a fill-degree order");
            };
            assert_eq!(degree_coefficient, expected_coefficient);
            assert_eq!(sample.seed, sample_seed(base_seed, replay_seed_index));
        }
    }

    assert_eq!(
        extra_sample(plan, 46).unwrap().seed,
        sample_seed(base_seed, 0)
    );
    assert_eq!(
        extra_sample(plan, 1_045).unwrap().seed,
        sample_seed(base_seed, 999)
    );
}

#[test]
fn the_hedge_leaves_every_restart_on_the_unmodified_sequence() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);
    let given = [1; 3];
    let ranked = vec![7, 1, 4];
    let base_seed = 17;
    // The portfolio ranks the modified pass's weights itself, so the caller's
    // are the plain pass.
    let fixed: Vec<(Order<'_>, u64)> = super::standard_orders(base_seed, &ranked)
        .into_iter()
        .filter(|candidate| super::reads_weights(candidate.order))
        .map(|candidate| (candidate.order, candidate.seed))
        .collect();
    assert_eq!(fixed.len(), 3, "nested dissection reads no weights");
    let cell = OnceCell::new();
    cell.set(ranked.clone()).expect("the cell is empty");
    let plan = hedged(base_seed, &graph, &cell, &given, fixed.len() as u64);
    let unmodified = schedule(base_seed, false, &given);

    // The plain diverse pass first, candidate for candidate.
    for index in 0..46 {
        let sample = extra_sample(plan, index).unwrap();
        let main = extra_sample(unmodified, index).unwrap();
        assert_eq!(sample.order, main.order, "diverse candidate {index}");
        assert_eq!(sample.seed, main.seed, "diverse candidate {index}");
        assert_eq!(sample.pass, Pass::Plain);
    }
    // Then the fixed orders that read the weights, labelled as the fixed
    // candidates they are and not as restarts.
    for (offset, &(order, seed)) in fixed.iter().enumerate() {
        let sample = extra_sample(plan, 46 + offset as u64).unwrap();
        assert_eq!(sample.order, order, "fixed order {offset}");
        assert_eq!(sample.seed, seed, "fixed order {offset}");
        assert_eq!(sample.pass, Pass::Modified);
        assert!(
            matches!(sample.stage, Stage::MinDegree | Stage::MinFill),
            "fixed order {offset} is not a restart: {:?}",
            sample.stage,
        );
    }
    // Then the diverse pass again on the ranking, on the seeds the plain one
    // used.
    for index in 0..46 {
        let sample = extra_sample(plan, 46 + 3 + index).unwrap();
        let main = extra_sample(unmodified, index).unwrap();
        let Order::FillDegreeSampled {
            degree_coefficient, ..
        } = main.order
        else {
            panic!("diverse candidate {index} did not use a fill-degree order");
        };
        assert_eq!(
            sample.order,
            Order::FillDegreeSampled {
                weights: &ranked,
                degree_coefficient,
            },
        );
        assert_eq!(sample.seed, main.seed);
        assert_eq!(sample.pass, Pass::Modified);
    }

    // Every restart is the one an unhedged portfolio runs, seed for seed.
    let first_restart = 46 + 3 + 46;
    for step in 0..1_000u64 {
        let sample = extra_sample(plan, first_restart + step).unwrap();
        let main = extra_sample(unmodified, 46 + step).unwrap();
        assert_eq!(sample.order, main.order, "restart {step}");
        assert_eq!(sample.seed, main.seed, "restart {step}");
        assert_eq!(sample.pass, Pass::Plain);
    }
    assert!(extra_sample(plan, first_restart + 1_000).is_none());
}

#[test]
fn the_ranking_is_placed_by_the_first_modified_candidate() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3)]);
    let given = [1; 4];
    let cell = OnceCell::new();
    let base_seed = 17;
    let plan = Schedule {
        modified: Some(ModifiedWeights {
            cell: &cell,
            graph: &graph,
            dim: 1,
            rounds: 8,
            seed: base_seed,
            soft_deadline: None,
        }),
        fixed_runs: 3,
        ..schedule(base_seed, false, &given)
    };

    // The plain pass draws nothing from the ranking.
    for index in 0..46 {
        assert_eq!(extra_sample(plan, index).unwrap().pass, Pass::Plain);
    }
    assert!(cell.get().is_none(), "the plain pass placed the ranking");
    // The first fixed order on the ranking places it and runs on it.
    let first = extra_sample(plan, 46).unwrap();
    let ranking = cell
        .get()
        .expect("the first modified candidate places the ranking");
    assert_eq!(ranking.len(), 4);
    assert_eq!(first.pass, Pass::Modified);
    assert_eq!(first.order, Order::MinDegreeSampled { weights: ranking });
    // The restarts after the modified pass stay on the caller's weights.
    let restart = extra_sample(plan, 46 + 3 + 46).unwrap();
    assert_eq!(restart.pass, Pass::Plain);
    assert_eq!(restart.order, Order::MinFillSampled { weights: &given });
}

#[test]
fn an_extra_sample_stops_at_the_soft_deadline() {
    let soft_deadline = Instant::now() + Duration::from_secs(1);
    let hard_deadline = soft_deadline + Duration::from_secs(1);
    let stop = elimination_stop(
        EliminationPhase::ExtraSampling,
        Some(soft_deadline),
        Some(hard_deadline),
        Some(17),
    );

    assert_eq!(stop.soft_deadline, Some(soft_deadline));
    assert_eq!(stop.hard_deadline, Some(soft_deadline));
    assert_eq!(stop.width_bound, Some(17));
}

/// The `side × side` grid, vertex `row * side + column`.
fn grid(side: u32) -> Graph {
    let mut edges = Vec::new();
    for row in 0..side {
        for column in 0..side {
            let vertex = row * side + column;
            if column + 1 < side {
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
fn a_candidate_stopped_at_the_soft_cutoff_does_not_end_the_portfolio() {
    // 900 vertices keeps the residual above CHEAP_MODE_MAX_ACTIVE, so the
    // deterministic min-degree candidate stops at the soft cutoff and the
    // engine completes what is left as one bag.
    let graph = grid(30);
    let weights = vec![0u32; graph.num_vertices() as usize];
    let config = PortfolioConfig::standard()
        .with_soft_budget(Duration::from_nanos(1))
        .with_hard_budget(Duration::from_secs(30))
        .with_flowcutter(Duration::from_secs(2))
        .with_hedge(Hedge::Off);

    let mut seen: Vec<(Stage, CandidateOutcome)> = Vec::new();
    let decomposition = super::decompose_traced(&graph, &weights, 0, config, &mut |candidate| {
        seen.push((candidate.stage, candidate.outcome));
    })
    .unwrap();

    decomposition.validate(&graph).unwrap();
    let (stage, outcome) = *seen
        .first()
        .expect("the portfolio runs its first candidate");
    assert_eq!(stage, Stage::MinDegree);
    assert!(
        matches!(outcome, CandidateOutcome::Produced { .. }),
        "the soft cutoff left a decomposition behind, so the candidate produced one: {outcome:?}"
    );
    assert!(
        seen.iter().any(|(stage, _)| *stage == Stage::FlowCutter),
        "the trailing FlowCutter slot still has the rest of the hard budget: {seen:?}"
    );
}
