use std::time::{Duration, Instant};

use super::PortfolioConfig;
use super::candidates::CandidateSet;
use super::{EliminationPhase, elimination_stop, extra_sample, sample_seed};
use crate::elimination::Order;
use crate::{Graph, TreeDecomposition};

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
    assert_eq!(config.diverse_sampling_runs, 9);
    assert_eq!(config.flowcutter_budget, Some(budget));
}

#[test]
fn an_explicit_hard_budget_does_not_change_the_soft_schedule() {
    let soft = Duration::from_millis(4_750);
    let hard = Duration::from_millis(9_000);
    let config = PortfolioConfig::standard_with_budget(soft).with_hard_budget(hard);

    assert_eq!(config.soft_budget, Some(soft));
    assert_eq!(config.hard_budget, Some(hard));
    assert_eq!(config.sampling_runs, 1_000);
    assert_eq!(config.diverse_sampling_runs, 9);
    assert_eq!(config.flowcutter_budget, Some(soft));
}

#[test]
fn diverse_samples_precede_the_complete_ordinary_schedule() {
    let weights = [1; 3];
    let expected_coefficients = [1, -1, -2, -3, -4, -5, -8, -7, -16];

    for (index, expected) in expected_coefficients.into_iter().enumerate() {
        let (order, _) = extra_sample(0, false, 1_000, 9, index as u64, &weights).unwrap();
        let Order::FillDegreeSampled {
            degree_coefficient, ..
        } = order
        else {
            panic!("diverse sample {index} did not use a fill-degree order");
        };
        assert_eq!(degree_coefficient, expected);
    }
    assert!(matches!(
        extra_sample(0, false, 1_000, 9, 9, &weights),
        Some((Order::MinFillSampled { .. }, _))
    ));
    assert!(matches!(
        extra_sample(0, false, 1_000, 9, 1_008, &weights),
        Some((Order::MinFillSampled { .. }, _))
    ));
    assert!(extra_sample(0, false, 1_000, 9, 1_009, &weights).is_none());
    assert!(matches!(
        extra_sample(0, true, 1_000, 9, 999, &weights),
        Some((Order::MinDegreeSampled { .. }, _))
    ));
    assert!(extra_sample(0, true, 1_000, 9, 1_000, &weights).is_none());
}

#[test]
fn diverse_samples_do_not_shift_the_ordinary_seed_stream() {
    let base_seed = 17;
    let weights = [1; 3];

    for index in 0..9 {
        let (_, diverse_seed) = extra_sample(base_seed, false, 1_000, 9, index, &weights).unwrap();
        assert_eq!(diverse_seed, sample_seed(base_seed, 0));
    }
    let (_, first_ordinary_seed) = extra_sample(base_seed, false, 1_000, 9, 9, &weights).unwrap();
    let (_, last_ordinary_seed) =
        extra_sample(base_seed, false, 1_000, 9, 1_008, &weights).unwrap();

    assert_eq!(first_ordinary_seed, sample_seed(base_seed, 0));
    assert_eq!(last_ordinary_seed, sample_seed(base_seed, 999));
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
