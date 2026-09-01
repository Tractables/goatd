use std::time::Duration;

use super::{Args, Method, portfolio_config};
use goatd::portfolio::PortfolioConfig;

#[test]
fn graph_loading_time_is_subtracted_from_the_portfolio_deadlines() {
    let soft = Duration::from_millis(4_750);
    let args = Args {
        input: "-".into(),
        out: None,
        order: Method::Portfolio,
        seed: None,
        sample: false,
        weights: None,
        budget: Some(soft),
        hard_budget: None,
        steps: None,
        refine: false,
    };

    let config = portfolio_config(&args, Duration::from_millis(300));
    let expected = PortfolioConfig::standard_with_budget(soft)
        .with_soft_budget(Duration::from_millis(4_450))
        .with_hard_budget(Duration::from_millis(9_200));

    assert_eq!(config, expected);
}
