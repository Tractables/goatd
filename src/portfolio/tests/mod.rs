use std::time::Duration;

use super::PortfolioConfig;

#[test]
fn a_short_standard_budget_keeps_the_fast_sampling_cap() {
    let budget = Duration::from_millis(4_999);
    let config = PortfolioConfig::standard_with_budget(budget);

    assert_eq!(config.soft_budget, Some(budget));
    assert_eq!(config.sampling_runs, 100);
}

#[test]
fn a_ten_second_hard_window_raises_the_standard_sampling_cap() {
    let budget = Duration::from_secs(5);
    let config = PortfolioConfig::standard_with_budget(budget);

    assert_eq!(config.soft_budget, Some(budget));
    assert_eq!(config.sampling_runs, 1_000);
}
