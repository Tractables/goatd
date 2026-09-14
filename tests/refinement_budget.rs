use std::time::{Duration, Instant};

use goatd::{Graph, TreeDecomposition, decomposition, meter};

#[test]
fn separator_search_obeys_the_refinement_work_deadline() {
    let graph = Graph::new(128, (0..127).map(|v| (v, v + 1)));
    let tree = TreeDecomposition::new(&graph, [(0..128).collect::<Vec<_>>()], []).unwrap();
    let _clock = meter::arm(Instant::now());
    let before = meter::units_spent();

    let result =
        decomposition::refine_with_flowcutter(tree, &graph, Some(Duration::from_millis(1)))
            .unwrap();

    result.validate(&graph).unwrap();
    let spent = meter::units_spent() - before;
    assert!(spent > 0, "the separator search must start");
    assert!(
        spent <= 2 * meter::UNITS_PER_MS,
        "a one-millisecond allocation spent {spent} work units"
    );
}
