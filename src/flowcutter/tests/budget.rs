use crate::flowcutter::{
    Budget, BudgetKind, FC_TIMED_STEPS, MAX_VERTICES, TimeoutBehavior, decompose,
};
use crate::{Error, Graph};
use std::time::Duration;

use super::super::duration_ms;

#[test]
fn a_positive_timed_budget_records_the_requested_wall_policy() {
    let budget = Budget::timed(
        Duration::from_millis(250),
        Some(Duration::from_millis(40)),
        12,
    );

    assert!(matches!(
        budget.kind,
        BudgetKind::Timed {
            timeout,
            patience: Some(patience),
            iterations: 12,
            steps: FC_TIMED_STEPS,
            timeout_behavior: TimeoutBehavior::AdaptSearch,
        } if timeout == Duration::from_millis(250)
            && patience == Duration::from_millis(40)
    ));
}

#[test]
fn a_positive_submillisecond_budget_reaches_the_vendor_as_one_millisecond() {
    assert_eq!(duration_ms(Duration::from_nanos(1)), 1);
}

#[test]
fn an_oversized_graph_is_refused_before_the_vendor_is_called() {
    let graph = Graph::new(MAX_VERTICES + 1, []);
    let error = decompose(&graph, Budget::steps(1, 1))
        .expect_err("the dense vendor representation cannot hold this graph");

    assert!(matches!(error, Error::TooLarge(_)));
    let message = error.to_string();
    assert!(message.contains("vertices") && message.contains(&(MAX_VERTICES + 1).to_string()));
}
