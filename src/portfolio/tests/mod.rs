use std::time::Duration;

use super::PortfolioConfig;
use super::candidates::CandidateSet;
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
fn a_short_standard_budget_keeps_the_fast_sampling_cap() {
    let budget = Duration::from_millis(4_749);
    let config = PortfolioConfig::standard_with_budget(budget);

    assert_eq!(config.soft_budget, Some(budget));
    assert_eq!(config.sampling_runs, 100);
    assert_eq!(config.flowcutter_budget, None);
}

#[test]
fn a_ten_second_outer_window_with_output_headroom_raises_the_sampling_cap() {
    let budget = Duration::from_millis(4_750);
    let config = PortfolioConfig::standard_with_budget(budget);

    assert_eq!(config.soft_budget, Some(budget));
    assert_eq!(config.sampling_runs, 1_000);
    assert_eq!(config.flowcutter_budget, Some(budget));
}
