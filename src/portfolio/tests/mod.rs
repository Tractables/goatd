use std::cell::OnceCell;
use std::time::{Duration, Instant};

use super::candidates::CandidateSet;
use super::config::{MAX_DIVERSE_SAMPLING_RUNS, validate};
use super::{CandidateOutcome, CommitPlan, DEFAULT_HEDGE_DIMS, Family, HedgeSeries, HedgeWeights};
use super::{EliminationPhase, Hedge, ModifiedWeights, Pass, PortfolioConfig, Sample, Schedule};
use super::{FLOWCUTTER_RESERVE, restart_admitted, restart_deadline};
use super::{Race, StageBudget};
use super::{Stage, elimination_stop, extra_sample, hedge_random_seed, sample_seed};
use crate::elimination::Order;
use crate::{Graph, TreeDecomposition};

/// An unhedged schedule: the caller's weights on every candidate, one diverse
/// pass, no weighted stage to run against it.
fn schedule(base_seed: u64, large_residual: bool, weights: &[u32]) -> Schedule<'_> {
    Schedule {
        base_seed,
        large_residual,
        ordinary_runs: 1_000,
        diverse_runs: 46,
        modified: &[],
        fixed_runs: 0,
        initial_orders: super::standard_orders,
        weights,
        commit: None,
        committed: None,
    }
}

/// A weighted stage whose ranking is already in `cell`, so the candidates can
/// be read without placing one.
fn placed<'a>(
    graph: &'a Graph,
    cell: &'a OnceCell<Vec<u32>>,
    ranking: &[u32],
) -> ModifiedWeights<'a> {
    cell.set(ranking.to_vec()).expect("the cell is empty");
    ModifiedWeights::Ranked {
        cell,
        graph,
        dim: 3,
        rounds: 1_000,
        seed: 0,
        deadline: None,
    }
}

/// A hedged schedule that runs `modified`, one weighted stage per entry.
fn hedged<'a>(
    base_seed: u64,
    modified: &'a [ModifiedWeights<'a>],
    weights: &'a [u32],
    fixed_runs: u64,
) -> Schedule<'a> {
    Schedule {
        base_seed,
        large_residual: false,
        ordinary_runs: 1_000,
        diverse_runs: 46,
        modified,
        fixed_runs,
        initial_orders: super::standard_orders,
        weights,
        commit: None,
        committed: None,
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
    assert!(config.restarts_to_deadline);
}

#[test]
fn a_ten_second_outer_window_with_output_headroom_raises_the_sampling_cap() {
    let budget = Duration::from_millis(4_750);
    let config = PortfolioConfig::standard_with_budget(budget);

    assert_eq!(config.soft_budget, Some(budget));
    assert_eq!(config.sampling_runs, 1_000);
    assert_eq!(config.diverse_sampling_runs, 46);
    assert_eq!(config.flowcutter_budget, Some(budget));
    assert!(config.restarts_to_deadline);
}

#[test]
fn only_the_budgeted_standard_portfolio_runs_its_restarts_to_the_deadline() {
    assert!(!PortfolioConfig::standard().restarts_to_deadline);
    assert!(!PortfolioConfig::sampled_min_fill().restarts_to_deadline);
    assert!(
        PortfolioConfig::standard()
            .with_restarts_to_deadline(true)
            .restarts_to_deadline
    );
    assert!(
        !PortfolioConfig::standard_with_budget(Duration::from_millis(4_750))
            .with_restarts_to_deadline(false)
            .restarts_to_deadline
    );
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
    let stage = |dim| HedgeWeights::Eccentricity { dim, rounds: 1_000 };
    let hedge = Hedge::Passes(
        HedgeSeries::of(stage(3))
            .then(stage(1))
            .then(stage(2))
            .then(stage(4))
            .then(stage(8))
            .then(stage(5))
            .then(stage(6))
            .then(stage(7)),
    );

    assert_eq!(DEFAULT_HEDGE_DIMS, [3, 1, 2, 4, 8, 5, 6, 7]);
    assert_eq!(Hedge::eccentricity(), hedge);
    assert_eq!(
        Hedge::Passes(HedgeSeries::eccentricity_dims(&DEFAULT_HEDGE_DIMS)),
        hedge,
        "the default hedge is one eccentricity stage per default dimension"
    );
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
    let placement = |dim, rounds| {
        PortfolioConfig::standard().with_hedge(Hedge::Passes(HedgeSeries::of(
            HedgeWeights::Eccentricity { dim, rounds },
        )))
    };

    assert!(validate(placement(0, 8)).is_err());
    assert!(validate(placement(9, 8)).is_err());
    assert!(validate(placement(2, 0)).is_err());
    assert!(validate(PortfolioConfig::standard()).is_ok());
}

#[test]
fn a_hedge_series_is_checked_before_it_runs() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);
    let weights = [1; 3];
    let refuse = |series: HedgeSeries, expected: &str| {
        let error = super::decompose(
            &graph,
            &weights,
            0,
            PortfolioConfig::standard().with_hedge(Hedge::Passes(series)),
        )
        .expect_err("the configuration is not usable");
        let message = error.to_string();
        assert!(
            message.contains(expected),
            "{message:?} does not mention {expected:?}",
        );
    };

    refuse(
        HedgeSeries::eccentricity_dims(&[]),
        "at least one modified pass",
    );
    refuse(HedgeSeries::eccentricity_dims(&[2, 3, 2]), "twice");
    refuse(HedgeSeries::random(9), "at most 8 modified passes");
    refuse(HedgeSeries::eccentricity_dims(&[9]), "outside 1..=8");
    assert!(
        super::decompose(
            &graph,
            &weights,
            0,
            PortfolioConfig::standard()
                .with_hedge(Hedge::Passes(HedgeSeries::eccentricity_dims(&[1, 2, 3]))),
        )
        .is_ok(),
        "three dimensions is a usable series",
    );
}

#[test]
fn a_series_runs_one_stage_per_weighting_it_was_given() {
    assert_eq!(
        HedgeSeries::eccentricity_dims(&[1, 2]).len(),
        2,
        "one stage per dimension",
    );
    assert_eq!(HedgeSeries::random(3).len(), 3, "one stage per draw");
    assert_eq!(HedgeSeries::of(HedgeWeights::eccentricity()).len(), 1);
    assert!(HedgeSeries::random(0).is_empty());
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
    let modified = [placed(&graph, &cell, &ranked)];
    let plan = hedged(base_seed, &modified, &given, fixed.len() as u64);
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
        assert_eq!(sample.pass, Pass::Modified { index: 0 });
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
        assert_eq!(sample.pass, Pass::Modified { index: 0 });
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
    let modified = [ModifiedWeights::Ranked {
        cell: &cell,
        graph: &graph,
        dim: 1,
        rounds: 8,
        seed: base_seed,
        deadline: None,
    }];
    let plan = Schedule {
        modified: &modified,
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
    assert_eq!(first.pass, Pass::Modified { index: 0 });
    assert_eq!(first.order, Order::MinDegreeSampled { weights: ranking });
    // The restarts after the modified pass stay on the caller's weights.
    let restart = extra_sample(plan, 46 + 3 + 46).unwrap();
    assert_eq!(restart.pass, Pass::Plain);
    assert_eq!(restart.order, Order::MinFillSampled { weights: &given });
}

#[test]
fn a_series_hedge_runs_one_weighted_stage_per_weighting() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);
    let given = [1; 3];
    let first = [7, 1, 4];
    let second = [4, 7, 1];
    let base_seed = 17;
    let (first_cell, second_cell) = (OnceCell::new(), OnceCell::new());
    let modified = [
        placed(&graph, &first_cell, &first),
        placed(&graph, &second_cell, &second),
    ];
    let plan = hedged(base_seed, &modified, &given, 3);
    let unmodified = schedule(base_seed, false, &given);

    // The plain diverse pass first, candidate for candidate, as under a hedge
    // of one weighting.
    for index in 0..46 {
        let sample = extra_sample(plan, index).unwrap();
        let main = extra_sample(unmodified, index).unwrap();
        assert_eq!(sample.order, main.order, "diverse candidate {index}");
        assert_eq!(sample.seed, main.seed, "diverse candidate {index}");
        assert_eq!(sample.pass, Pass::Plain);
    }
    // Then a stage per weighting: its fixed orders, then its diverse pass, on
    // the seeds the plain pass used.
    for (stage, weights) in [(0u64, &first), (1, &second)] {
        let stage_start = 46 + stage * (3 + 46);
        let fixed: Vec<(Order<'_>, u64)> = super::standard_orders(base_seed, weights)
            .into_iter()
            .filter(|candidate| super::reads_weights(candidate.order))
            .map(|candidate| (candidate.order, candidate.seed))
            .collect();
        assert_eq!(fixed.len(), 3);
        for (offset, &(order, seed)) in fixed.iter().enumerate() {
            let sample = extra_sample(plan, stage_start + offset as u64).unwrap();
            assert_eq!(sample.order, order, "stage {stage} fixed order {offset}");
            assert_eq!(sample.seed, seed, "stage {stage} fixed order {offset}");
            assert_eq!(sample.pass, Pass::Modified { index: stage as u8 });
        }
        for index in 0..46 {
            let sample = extra_sample(plan, stage_start + 3 + index).unwrap();
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
                    weights,
                    degree_coefficient,
                },
                "stage {stage} diverse candidate {index}",
            );
            assert_eq!(sample.seed, main.seed, "stage {stage} candidate {index}");
            assert_eq!(sample.pass, Pass::Modified { index: stage as u8 });
        }
    }
    // Every restart is still the one an unhedged portfolio runs, seed for seed,
    // and there are as many as ever.
    let first_restart = 46 + 2 * (3 + 46);
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
fn a_random_weighting_is_drawn_by_its_own_first_candidate() {
    let given = [1; 4];
    let base_seed = 17;
    let cells: [OnceCell<Vec<u32>>; 2] = std::array::from_fn(|_| OnceCell::new());
    let modified = [
        ModifiedWeights::Random {
            cell: &cells[0],
            count: 4,
            seed: hedge_random_seed(base_seed, 0),
        },
        ModifiedWeights::Random {
            cell: &cells[1],
            count: 4,
            seed: hedge_random_seed(base_seed, 1),
        },
    ];
    let plan = hedged(base_seed, &modified, &given, 3);

    for index in 0..46 {
        assert_eq!(extra_sample(plan, index).unwrap().pass, Pass::Plain);
    }
    assert!(
        cells.iter().all(|cell| cell.get().is_none()),
        "the plain pass drew weights it does not use",
    );
    let first = extra_sample(plan, 46).unwrap();
    let drawn = cells[0].get().expect("the first stage draws its weights");
    assert_eq!(first.pass, Pass::Modified { index: 0 });
    assert_eq!(first.order, Order::MinDegreeSampled { weights: drawn });
    assert!(
        cells[1].get().is_none(),
        "the second stage drew before it ran",
    );
    let second = extra_sample(plan, 46 + 3 + 46).unwrap();
    let next = cells[1].get().expect("the second stage draws its weights");
    assert_eq!(second.pass, Pass::Modified { index: 1 });
    assert_eq!(second.order, Order::MinDegreeSampled { weights: next });
    assert_ne!(drawn, next, "both stages drew the same weights");
    assert_ne!(drawn.as_slice(), given.as_slice());
}

#[test]
fn a_random_weighting_draws_off_every_other_stream() {
    // The seeds are the run's, one stride apart, and none of them lands on a
    // restart's seed.
    assert_eq!(hedge_random_seed(17, 0), 17 + 6_151);
    assert_eq!(hedge_random_seed(17, 1), 17 + 6_151 + 104_729);
    for stream in 0..8u64 {
        for sample_index in 0..2_000u64 {
            assert_ne!(hedge_random_seed(17, stream), sample_seed(17, sample_index));
        }
    }
}

/// `seconds` as a duration, for the stage-budget arithmetic.
fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

#[test]
fn a_weighted_stage_runs_only_while_the_reserve_holds_one_more() {
    // A plain pass of a second, ten seconds of soft budget left, half of them
    // for the stages after the first: five stages of a second each fit and the
    // sixth does not.
    let mut budget = StageBudget::new(secs(1), Some(secs(10)), 0.5);

    for stage in 0..5 {
        assert!(budget.fits(), "stage {stage} fits in the reserve");
        budget.charge(secs(1));
    }

    assert!(!budget.fits(), "the sixth stage overruns the reserve");
    assert_eq!(
        budget.refusal(),
        CandidateOutcome::StageSkipped {
            projected: secs(1),
            spent: secs(5),
            allowance: secs(5),
        },
    );
}

#[test]
fn a_plain_pass_that_nearly_filled_the_budget_runs_the_first_stage_and_no_more() {
    // 9.2 s of a 30 s budget on the plain pass. The first stage runs whatever
    // the reserve is; it costs 9.2 s as well, and against the 9.4 s half the
    // remainder leaves, a second stage does not fit.
    let mut budget = StageBudget::new(
        Duration::from_millis(9_200),
        Some(Duration::from_millis(18_800)),
        0.5,
    );

    assert!(
        budget.fits(),
        "a hedge runs one weighted stage on any budget"
    );
    budget.charge(Duration::from_millis(9_200));

    assert!(!budget.fits());
    assert_eq!(
        budget.refusal(),
        CandidateOutcome::StageSkipped {
            projected: Duration::from_millis(9_200),
            spent: Duration::from_millis(9_200),
            allowance: Duration::from_millis(9_400),
        },
    );
}

#[test]
fn without_a_soft_budget_every_stage_of_the_series_runs() {
    // No soft budget, so there is no restart time the stages could take, and an
    // expensive plain pass says nothing about how many of them run. All eight
    // stages of the default series run, and that stays cheap: without a
    // deadline the diverse pass does not run either, so a stage there is the
    // fixed orders that read weights and nothing else.
    let mut budget = StageBudget::new(Duration::from_secs(600), None, 0.5);

    for stage in 0..super::MAX_HEDGE_PASSES {
        assert!(budget.fits(), "stage {stage} of the series must run");
        budget.charge(Duration::from_secs(600));
    }
}

#[test]
fn a_stage_cheaper_than_the_plain_pass_is_what_the_next_one_is_projected_at() {
    // Four seconds on the plain pass, 4.5 s of reserve.
    let mut budget = StageBudget::new(secs(4), Some(secs(9)), 0.5);
    assert_eq!(budget.projected(), secs(4));
    assert!(budget.fits());

    // The incumbent bounded the first stage, so it cost a second, and the
    // second stage is projected at that rather than at the plain pass — which
    // is what lets it run at all.
    budget.charge(secs(1));
    assert_eq!(budget.projected(), secs(1));
    assert!(budget.fits());

    // A stage longer than the plain pass does not raise the projection above
    // it, and the spend is what refuses the next one.
    budget.charge(secs(6));
    assert_eq!(budget.projected(), secs(4));
    assert!(!budget.fits());
}

#[test]
fn a_run_with_no_soft_budget_bounds_no_stage() {
    let mut budget = StageBudget::new(secs(30), None, 0.5);

    for stage in 0..super::MAX_HEDGE_PASSES {
        assert!(budget.fits(), "stage {stage} has nothing to overrun");
        budget.charge(secs(30));
    }
}

#[test]
fn a_reserve_outside_the_unit_interval_is_refused() {
    let refuse = |fraction: f64| {
        let message = validate(PortfolioConfig::standard().with_hedge_reserve(fraction))
            .expect_err("the configuration is not usable")
            .to_string();
        assert!(
            message.contains("hedge reserve"),
            "{message:?} does not mention the reserve",
        );
    };

    refuse(0.0);
    refuse(-0.5);
    refuse(1.5);
    refuse(f64::NAN);
    assert!(
        validate(PortfolioConfig::standard().with_hedge_reserve(1.0)).is_ok(),
        "the whole of what is left is a usable reserve",
    );
}

#[test]
fn an_extra_sample_stops_at_the_restart_deadline() {
    let restart_deadline = Instant::now() + Duration::from_secs(1);
    let hard_deadline = restart_deadline + Duration::from_secs(1);
    let stop = elimination_stop(
        EliminationPhase::ExtraSampling,
        Some(restart_deadline),
        Some(hard_deadline),
        Some(17),
        false,
    );

    assert_eq!(stop.soft_deadline, Some(restart_deadline));
    assert_eq!(stop.hard_deadline, Some(restart_deadline));
    assert_eq!(stop.width_bound, Some(17));
    assert!(!stop.abort_on_tie);
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

/// A ring with two chords per vertex: cheap to build, and sparse enough that
/// its size rather than its density is what makes FlowCutter slow on it.
fn ring_with_chords(vertices: u32) -> Graph {
    let mut edges = Vec::with_capacity(3 * vertices as usize);
    for vertex in 0..vertices {
        edges.push((vertex, (vertex + 1) % vertices));
        edges.push((vertex, (vertex + 7) % vertices));
        edges.push((vertex, (vertex + 53) % vertices));
    }
    Graph::new(vertices, edges)
}

#[test]
fn the_flowcutter_slot_declines_a_graph_it_could_not_stop_on() {
    // A 20x20 grid needs a few milliseconds of setup and a restart, so even a
    // 200-millisecond window is enough for it.
    let small = grid(20);
    let candidate = super::flowcutter_candidate(&small, Duration::from_millis(200), None).unwrap();
    assert!(candidate.is_some(), "a small graph fits a short window");

    // 60,000 vertices and 180,000 edges put setup and one restart at about
    // nine seconds, and the backend cannot stop before finishing that restart.
    let large = ring_with_chords(60_000);
    let candidate =
        super::flowcutter_candidate(&large, Duration::from_millis(4_750), None).unwrap();
    assert!(
        candidate.is_none(),
        "a graph whose first restart outlasts the window is declined"
    );
}

#[test]
fn the_restarts_keep_a_flowcutter_reserve_at_the_end_of_the_hard_window() {
    let start = crate::meter::now();
    let soft = start + secs(5);
    let hard = start + secs(10);

    // An ordinary residual: the restarts run into the hard window and stop a
    // reserve short of its end.
    assert_eq!(
        restart_deadline(false, Some(soft), Some(hard)),
        Some(hard - FLOWCUTTER_RESERVE),
    );

    // A large residual keeps the soft deadline.
    assert_eq!(restart_deadline(true, Some(soft), Some(hard)), Some(soft),);

    // A hard window shorter than the reserve would put the restarts before the
    // soft deadline, so the soft deadline stands.
    let tight = start + Duration::from_millis(5_500);
    assert_eq!(restart_deadline(false, Some(soft), Some(tight)), Some(soft),);

    // No budget, so no hard window to run into.
    assert_eq!(restart_deadline(false, None, None), None);
}

#[test]
fn a_restart_that_would_not_finish_in_time_is_not_started() {
    let now = crate::meter::now();
    let restart = Some(now + Duration::from_millis(500));
    let hard = Some(now + secs(2));

    assert!(
        restart_admitted(now, Duration::from_millis(400), [restart, hard]),
        "a restart that ends before both deadlines is admitted",
    );
    assert!(
        !restart_admitted(now, Duration::from_millis(600), [restart, hard]),
        "a restart that runs into the restart deadline is not",
    );
    assert!(
        !restart_admitted(now, secs(3), [None, hard]),
        "nor is one that runs past the hard deadline",
    );
    assert!(
        restart_admitted(now, secs(30), [None, None]),
        "with no deadline the count is what stops the restarts",
    );
}

#[test]
fn the_flowcutter_window_stops_one_restart_short_of_the_hard_deadline() {
    let configured = Duration::from_millis(4_750);
    let one_restart = Duration::from_millis(170);

    assert_eq!(
        super::flowcutter_window(configured, Some(Duration::from_millis(1_500)), one_restart),
        Duration::from_millis(1_330),
        "the backend is stopped a restart before the hard deadline",
    );
    assert_eq!(
        super::flowcutter_window(configured, Some(secs(9)), one_restart),
        configured - one_restart,
        "a window wider than the configured budget is capped by it",
    );
    assert_eq!(
        super::flowcutter_window(configured, Some(Duration::from_millis(100)), one_restart),
        Duration::ZERO,
        "a window shorter than one restart leaves nothing",
    );
    assert_eq!(
        super::flowcutter_window(configured, None, one_restart),
        configured,
        "with no hard deadline the configured budget stands",
    );
}

/// Families for the commit-rule tests: two diverse coefficients and the
/// ordinary restart.
const RACED: [Family; 3] = [Family::Diverse(1), Family::Diverse(-1), Family::MinFill];

#[test]
fn the_race_commits_to_the_family_with_the_smallest_mean_width() {
    let mut race = Race::new(RACED.len(), 0);
    // The second family is the narrowest on average; the third is far enough
    // behind after the first round to be dropped, and is drawn no more.
    for round in [[10, 9, 40], [10, 8, 40], [10, 12, 40]] {
        for (family, width) in round.into_iter().enumerate() {
            if race.alive[family] {
                race.record(family, Some(width));
            }
        }
        race.drop_outclassed();
    }

    assert!(
        !race.alive[2],
        "a family more than half again the best width is dropped",
    );
    assert_eq!(race.samples[2], 1, "a dropped family is drawn no more");
    assert_eq!(race.winner(&RACED), Some(Family::Diverse(-1)));
}

#[test]
fn a_race_nothing_produced_in_commits_to_nothing() {
    let mut race = Race::new(RACED.len(), 0);
    for family in 0..RACED.len() {
        race.record(family, None);
    }
    race.drop_outclassed();

    assert!(race.alive.iter().all(|&alive| alive));
    assert_eq!(
        race.winner(&RACED),
        None,
        "the restarts stay on the ordinary sequence",
    );
}

#[test]
fn a_tie_on_mean_width_is_broken_by_the_run_seed() {
    let winners: Vec<Option<Family>> = (0..16)
        .map(|seed| {
            let mut race = Race::new(RACED.len(), seed);
            for family in 0..RACED.len() {
                race.record(family, Some(7));
            }
            race.winner(&RACED)
        })
        .collect();

    assert!(winners.iter().all(Option::is_some));
    assert!(
        winners.windows(2).any(|pair| pair[0] != pair[1]),
        "a different seed can pick a different family out of a tie",
    );
}

#[test]
fn the_race_draws_every_family_each_round_and_commits_the_tail() {
    let weights = [1; 3];
    let families = [Family::Diverse(-3), Family::MinFill];
    let mut plan = schedule(5, false, &weights);
    plan.commit = Some(CommitPlan {
        families: &families,
        rounds: 2,
    });

    for round in 0..2u64 {
        let diverse = extra_sample(plan, round * 2).unwrap();
        assert!(matches!(
            diverse.order,
            Order::FillDegreeSampled {
                degree_coefficient: -3,
                ..
            }
        ));
        assert_eq!(diverse.seed, sample_seed(5, round));
        let ordinary = extra_sample(plan, round * 2 + 1).unwrap();
        assert!(matches!(ordinary.order, Order::MinFillSampled { .. }));
        assert_eq!(ordinary.seed, sample_seed(5, round));
    }

    let uncommitted = extra_sample(plan, 4).unwrap();
    assert!(
        matches!(uncommitted.order, Order::MinFillSampled { .. }),
        "with nothing committed the restarts stay on the ordinary sequence",
    );
    assert_eq!(
        uncommitted.seed,
        sample_seed(5, 2),
        "the tail carries on from the seed after the last round",
    );

    plan.committed = Some(Family::Diverse(-3));
    let committed = extra_sample(plan, 4).unwrap();
    assert!(matches!(
        committed.order,
        Order::FillDegreeSampled {
            degree_coefficient: -3,
            ..
        }
    ));
    assert_eq!(
        committed.stage,
        Stage::Diverse {
            degree_coefficient: -3
        }
    );
}

#[test]
fn a_commit_rule_with_nothing_to_race_is_rejected() {
    let budgeted = PortfolioConfig::standard_with_budget(Duration::from_secs(10));
    assert!(
        validate(budgeted.with_restart_commit(0)).is_err(),
        "a race of no rounds decides nothing",
    );
    assert!(
        validate(PortfolioConfig::standard().with_restart_commit(3)).is_err(),
        "without the diverse orders the mix is one family",
    );
    validate(budgeted.with_restart_commit(3)).unwrap();
}
