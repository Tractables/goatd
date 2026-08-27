use crate::flowcutter::{FC_MAX_VERTICES, FC_TIMED_STEPS, FcBudget, WallCapMode, flowcutter_td};
use crate::{Error, Graph};

#[test]
fn a_positive_timed_budget_records_the_requested_wall_policy() {
    let budget = FcBudget::timed(250, 40, 12);

    assert!(matches!(
        budget,
        FcBudget::Timed {
            timeout_ms: 250,
            patience_ms: 40,
            iters: 12,
            steps: FC_TIMED_STEPS,
            cap_mode: WallCapMode::Tight,
        }
    ));
}

#[test]
fn a_nonpositive_timed_budget_becomes_a_reproducible_step_budget() {
    for timeout_ms in [0, -1] {
        assert!(matches!(
            FcBudget::timed(timeout_ms, 40, 12),
            FcBudget::Steps {
                steps: FC_TIMED_STEPS,
                iters: 12,
            }
        ));
    }
}

#[test]
fn an_oversized_graph_is_refused_before_the_vendor_is_called() {
    let graph = Graph::new(FC_MAX_VERTICES + 1, []);
    let error = flowcutter_td(&graph, FcBudget::Steps { steps: 1, iters: 1 })
        .expect_err("the dense vendor representation cannot hold this graph");

    assert!(matches!(error, Error::TooLarge(_)));
    let message = error.to_string();
    assert!(message.contains("vertices") && message.contains(&(FC_MAX_VERTICES + 1).to_string()));
}
