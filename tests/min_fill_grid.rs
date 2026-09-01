use std::time::{Duration, Instant};

use goatd::elimination::{Order, decompose};
use goatd::{Graph, meter};

fn grid(side: u32) -> Graph {
    let edges = (0..side)
        .flat_map(|row| {
            (0..side).flat_map(move |column| {
                let vertex = row * side + column;
                [
                    (column + 1 < side).then_some((vertex, vertex + 1)),
                    (row + 1 < side).then_some((vertex, vertex + side)),
                ]
                .into_iter()
                .flatten()
            })
        })
        .collect::<Vec<_>>();
    Graph::new(side * side, edges)
}

#[test]
fn min_fill_returns_width_at_most_twenty_nine_on_a_twenty_by_twenty_grid() {
    let graph = grid(20);
    let decomposition = decompose(&graph, Order::MinFill, 0, None).unwrap();

    decomposition.validate(&graph).unwrap();
    assert!(
        decomposition.treewidth() <= 29,
        "min-fill returned width {}",
        decomposition.treewidth(),
    );
}

#[test]
fn budgeted_min_fill_keeps_a_forty_by_forty_grid_compact() {
    let graph = grid(40);
    let _guard = meter::arm(Instant::now());
    let decomposition =
        decompose(&graph, Order::MinFill, 0, Some(Duration::from_millis(100))).unwrap();

    decomposition.validate(&graph).unwrap();
    assert!(
        decomposition.treewidth() <= 100,
        "budgeted min-fill returned width {}",
        decomposition.treewidth(),
    );
    assert!(
        decomposition.total_bag_size() <= 100_000,
        "budgeted min-fill returned total bag size {}",
        decomposition.total_bag_size(),
    );
}
