use goatd::Graph;
use goatd::flowcutter::separator::{self, Budget};
use std::time::Duration;

#[test]
fn separator_on_path_graph_is_small() {
    let n = 10;
    let edges: Vec<(u32, u32)> = (0..9).map(|i| (i, i + 1)).collect();
    let graph = Graph::new(n as u32, edges);
    let r = separator::find(&graph, Budget::new(10_000, 3))
        .expect("valid separator budget")
        .expect("separator on path");
    assert_eq!(r.vertices().len() + r.side_a().len() + r.side_b().len(), n);
    assert!(!r.side_a().is_empty());
    assert!(!r.side_b().is_empty());
    assert!(
        r.vertices().len() <= 2,
        "path separator too big: {:?}",
        r.vertices()
    );
}

#[test]
fn separator_on_a_disconnected_graph_returns_none() {
    // Two disjoint triangles need no separator to be disconnected.
    let n = 6;
    let edges = vec![(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)];
    let graph = Graph::new(n as u32, edges);
    let result = separator::find(&graph, Budget::new(10_000, 3)).expect("valid separator budget");
    assert!(result.is_none());
}

#[test]
fn separator_search_rejects_empty_work_budgets() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);
    for config in [
        Budget::new(0, 1),
        Budget::new(1, 0),
        Budget::new(1, 1).with_timeout(Duration::ZERO),
    ] {
        assert!(separator::find(&graph, config).is_err());
    }
}

#[test]
fn separator_search_rejects_unrepresentable_work_limits() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);
    for budget in [
        Budget::new(i64::MAX as u64 + 1, 1),
        Budget::new(1, i32::MAX as u32 + 1),
        Budget::new(1, 1).with_timeout(Duration::MAX),
    ] {
        assert!(separator::find(&graph, budget).is_err());
    }
}

#[test]
fn separator_search_rejects_an_unrepresentable_expanded_graph() {
    let graph = Graph::new(u32::MAX / 2 + 1, []);

    assert!(separator::find(&graph, Budget::new(1, 1)).is_err());
}
