//! The shared elimination loop and its width bound.

use crate::elimination::execution::{ElimExit, ElimSink, ElimStop, eliminate_in_order};
use crate::elimination::graph::EliminationGraph;

#[test]
fn an_in_order_run_stops_at_the_width_bound_before_adding_fill() {
    // A star: eliminating the hub first would make its leaves a clique, a
    // fill quadratic in the degree that a run past the bound never uses.
    let leaves = 64u32;
    let edges: Vec<(u32, u32)> = (1..=leaves).map(|leaf| (0, leaf)).collect();
    let mut graph = EliminationGraph::from_edges(leaves + 1, &edges);
    let mut bags = Vec::new();
    let mut ranks = Vec::new();
    let mut sink = ElimSink::new(&mut bags, &mut ranks, 0);
    let stop = ElimStop {
        soft_deadline: None,
        hard_deadline: None,
        width_bound: Some(2),
    };

    let exit = eliminate_in_order(&mut graph, 0..=leaves, &mut sink, stop);

    assert_eq!(exit, ElimExit::WidthLimitExceeded);
    assert!(bags.is_empty(), "no bag past the bound is recorded");
    assert!(graph.active[0], "the hub is not eliminated");
    assert_eq!(graph.num_edges, leaves as usize, "no fill edge is added");
    for leaf in 1..=leaves {
        assert_eq!(graph.degree(leaf), 1);
    }
}
