//! Test-side entry point into the search shell.
//!
//! These tests drive single `(order, seed)` pairs straight from raw edges.
//! Production never does — it always goes through `prebuild` +
//! `run_order_prebuilt`, which amortize graph construction and preprocessing
//! across a whole portfolio — so the raw-edges entry point lives here, next to
//! the module's private internals it needs.

use crate::elimination::Order;
use crate::elimination::engine::*;
use crate::elimination::execution::ElimStop;
use crate::elimination::graph::EliminationGraph;
use crate::elimination::preprocess::preprocess;

fn complete_bipartite(side: u32) -> crate::Graph {
    let mut edges = Vec::new();
    for left in 0..side {
        for right in side..(2 * side) {
            edges.push((left, right));
        }
    }
    crate::Graph::new(2 * side, edges)
}

fn disjoint_complete_bipartite(side: u32) -> crate::Graph {
    let component_size = 2 * side;
    let mut edges = Vec::new();
    for offset in [0, component_size] {
        for left in offset..(offset + side) {
            for right in (offset + side)..(offset + component_size) {
                edges.push((left, right));
            }
        }
    }
    crate::Graph::new(2 * component_size, edges)
}

fn complete_at_immediate_deadline(graph: &crate::Graph) -> crate::TreeDecomposition {
    let mut prebuilt = prebuild(graph);
    let epoch = std::time::Instant::now();
    let _meter = crate::meter::arm(epoch);
    let run = run_order_prebuilt(
        &mut prebuilt,
        RunSpec {
            order: Order::MinDegree,
            seed: 0,
            stop: ElimStop {
                soft_deadline: None,
                hard_deadline: Some(epoch),
                width_bound: None,
            },
            complete_on_deadline: true,
        },
    );
    let OrderRun::CompletedAtDeadline(decomposition) = run else {
        panic!("deadline completion must return its completed decomposition");
    };
    decomposition
}

/// Run a single `(order, seed)` pair from raw edges. Builds the graph and
/// runs preprocessing.
pub(super) fn run_order(
    num_vertices: u32,
    edges: &[(u32, u32)],
    order: Order<'_>,
    seed: u64,
) -> crate::TreeDecomposition {
    let graph = EliminationGraph::from_edges(num_vertices, edges);
    let reduced = preprocess(graph);
    let components = find_connected_components(&reduced.graph);
    match run_order_on_reduced(
        reduced,
        &components,
        None,
        RunSpec {
            order,
            seed,
            stop: ElimStop::default(),
            complete_on_deadline: false,
        },
    ) {
        OrderRun::Completed(decomposition) => decomposition,
        OrderRun::CompletedAtDeadline(_) | OrderRun::DeadlineAborted | OrderRun::WidthAborted => {
            unreachable!("an unbounded run has no cutoff")
        }
    }
}

#[test]
fn partial_eliminations_are_never_returned_as_decompositions() {
    let graph = complete_bipartite(40);
    let mut prebuilt = prebuild(&graph);
    let epoch = std::time::Instant::now();
    let _meter = crate::meter::arm(epoch);

    let mut at_deadline = |complete_on_deadline| {
        run_order_prebuilt(
            &mut prebuilt,
            RunSpec {
                order: Order::MinDegree,
                seed: 0,
                stop: ElimStop {
                    soft_deadline: None,
                    hard_deadline: Some(epoch),
                    width_bound: None,
                },
                complete_on_deadline,
            },
        )
    };

    assert!(matches!(at_deadline(false), OrderRun::DeadlineAborted));
    let OrderRun::CompletedAtDeadline(decomposition) = at_deadline(true) else {
        panic!("deadline completion must return its completed decomposition");
    };
    decomposition.validate(&graph).unwrap();

    let width_limited = run_order_prebuilt(
        &mut prebuilt,
        RunSpec {
            order: Order::MinDegree,
            seed: 0,
            stop: ElimStop {
                soft_deadline: None,
                hard_deadline: None,
                width_bound: Some(0),
            },
            complete_on_deadline: false,
        },
    );
    assert!(matches!(width_limited, OrderRun::WidthAborted));
}

#[test]
fn deadline_completion_does_not_emit_one_bag_per_residual_vertex() {
    let graph = complete_bipartite(40);
    let decomposition = complete_at_immediate_deadline(&graph);

    decomposition.validate(&graph).unwrap();
    assert!(decomposition.bags().len() < graph.num_vertices() as usize);
}

#[test]
fn deadline_completion_keeps_unfinished_components_in_separate_bags() {
    let graph = disjoint_complete_bipartite(40);
    let decomposition = complete_at_immediate_deadline(&graph);

    decomposition.validate(&graph).unwrap();
    assert!(decomposition.bags().len() < graph.num_vertices() as usize);
    assert_eq!(decomposition.bags().last().unwrap().vertices().len(), 80);
}
