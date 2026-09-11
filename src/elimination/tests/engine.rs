//! Test-side entry point into the search shell.
//!
//! These tests drive single `(order, seed)` pairs straight from raw edges.
//! Production never does — it always goes through `prebuild` +
//! `run_order_prebuilt`, which amortize graph construction and preprocessing
//! across a whole portfolio — so the raw-edges entry point lives here, next to
//! the module's private internals it needs.

use crate::elimination::Order;
use crate::elimination::engine::*;
use crate::elimination::execution::{Cutoff, ElimStop};
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

/// Two square grids with nothing between them. A grid has no simplicial
/// vertex, so preprocessing leaves both components for the elimination.
fn disjoint_grids(side: u32) -> crate::Graph {
    let component_size = side * side;
    let mut edges = Vec::new();
    for offset in [0, component_size] {
        for row in 0..side {
            for column in 0..side {
                let vertex = offset + row * side + column;
                if column + 1 < side {
                    edges.push((vertex, vertex + 1));
                }
                if row + 1 < side {
                    edges.push((vertex, vertex + side));
                }
            }
        }
    }
    crate::Graph::new(2 * component_size, edges)
}

/// A chorded ring: degree six at every vertex and no simplicial one, so
/// preprocessing hands the whole graph on to the elimination.
fn chorded_ring(vertices: u32) -> crate::Graph {
    let mut edges = Vec::with_capacity(3 * vertices as usize);
    for vertex in 0..vertices {
        edges.push((vertex, (vertex + 1) % vertices));
        edges.push((vertex, (vertex + 7) % vertices));
        edges.push((vertex, (vertex + 53) % vertices));
    }
    crate::Graph::new(vertices, edges)
}

fn complete_at_immediate_deadline(
    graph: &crate::Graph,
    order: Order<'_>,
) -> crate::TreeDecomposition {
    let mut prebuilt = prebuild(graph, None);
    let epoch = std::time::Instant::now();
    let _meter = crate::meter::arm(epoch);
    let run = run_order_prebuilt(
        &mut prebuilt,
        RunSpec {
            order,
            seed: 0,
            sample_band: 0,
            update_order_ties: false,
            stop: ElimStop {
                soft_deadline: None,
                hard_deadline: Some(epoch),
                width_bound: None,
            },
            complete_on_deadline: true,
            setup_deadline: None,
        },
    );
    let OrderRun::CompletedAtDeadline(_, decomposition) = run else {
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
    let mut reduced = preprocess(graph, None);
    let components = find_connected_components(&reduced.graph);
    match run_order_on_residual(
        &mut reduced.graph,
        &reduced.prefix,
        &components,
        None,
        None,
        RunSpec {
            order,
            seed,
            sample_band: 0,
            update_order_ties: false,
            stop: ElimStop::default(),
            complete_on_deadline: false,
            setup_deadline: None,
        },
        &mut RunScratch::new(),
    ) {
        OrderRun::Completed(decomposition) => decomposition,
        OrderRun::CompletedAtDeadline(..)
        | OrderRun::DeadlineAborted(_)
        | OrderRun::WidthAborted => {
            unreachable!("an unbounded run has no cutoff")
        }
    }
}

#[test]
fn partial_eliminations_are_never_returned_as_decompositions() {
    let graph = complete_bipartite(40);
    let mut prebuilt = prebuild(&graph, None);
    let epoch = std::time::Instant::now();
    let _meter = crate::meter::arm(epoch);

    let mut at_deadline = |complete_on_deadline| {
        run_order_prebuilt(
            &mut prebuilt,
            RunSpec {
                order: Order::MinDegree,
                seed: 0,
                sample_band: 0,
                update_order_ties: false,
                stop: ElimStop {
                    soft_deadline: None,
                    hard_deadline: Some(epoch),
                    width_bound: None,
                },
                complete_on_deadline,
                setup_deadline: None,
            },
        )
    };

    assert!(matches!(
        at_deadline(false),
        OrderRun::DeadlineAborted(Cutoff::Hard)
    ));
    let OrderRun::CompletedAtDeadline(Cutoff::Hard, decomposition) = at_deadline(true) else {
        panic!("deadline completion must return its completed decomposition");
    };
    decomposition.validate(&graph).unwrap();

    let width_limited = run_order_prebuilt(
        &mut prebuilt,
        RunSpec {
            order: Order::MinDegree,
            seed: 0,
            sample_band: 0,
            update_order_ties: false,
            stop: ElimStop {
                soft_deadline: None,
                hard_deadline: None,
                width_bound: Some(0),
            },
            complete_on_deadline: false,
            setup_deadline: None,
        },
    );
    assert!(matches!(width_limited, OrderRun::WidthAborted));
}

#[test]
fn deadline_completion_does_not_emit_one_bag_per_residual_vertex() {
    let graph = complete_bipartite(40);
    let decomposition = complete_at_immediate_deadline(&graph, Order::MinDegree);

    decomposition.validate(&graph).unwrap();
    assert!(decomposition.bags().len() < graph.num_vertices() as usize);
}

#[test]
fn a_soft_cutoff_leaves_the_components_after_it_their_own_orders() {
    // The first grid is well above CHEAP_MODE_MAX_ACTIVE when the soft cutoff
    // is checked, so it stops there with most of itself in one bag. The second
    // grid still has the whole hard deadline, and the vertices from 900 up are
    // exactly that second grid.
    let graph = disjoint_grids(30);
    let mut prebuilt = prebuild(&graph, None);
    let epoch = std::time::Instant::now();
    let _meter = crate::meter::arm(epoch);
    let run = run_order_prebuilt(
        &mut prebuilt,
        RunSpec {
            order: Order::MinDegree,
            seed: 0,
            sample_band: 0,
            update_order_ties: false,
            stop: ElimStop {
                soft_deadline: Some(epoch),
                hard_deadline: Some(epoch + std::time::Duration::from_secs(30)),
                width_bound: None,
            },
            complete_on_deadline: true,
            setup_deadline: None,
        },
    );
    let OrderRun::CompletedAtDeadline(Cutoff::Soft, decomposition) = run else {
        panic!("the soft cutoff completes the residual and reports itself");
    };
    decomposition.validate(&graph).unwrap();

    let widest_second_component = decomposition
        .bags()
        .iter()
        .map(|bag| bag.vertices().iter().filter(|&&v| v >= 900).count())
        .max()
        .expect("a decomposition has bags");
    // Preprocessing takes the four corners of each grid, so the second
    // component reaches the elimination as 896 vertices and is that one bag
    // when it is bagged whole. Decomposed, its bags are around 30.
    assert!(
        widest_second_component < 100,
        "the second grid is decomposed rather than bagged whole, \
         but one bag holds {widest_second_component} of its vertices"
    );
}

#[test]
fn deadline_completion_keeps_unfinished_components_in_separate_bags() {
    let graph = disjoint_complete_bipartite(40);
    let decomposition = complete_at_immediate_deadline(&graph, Order::MinDegree);

    decomposition.validate(&graph).unwrap();
    assert!(decomposition.bags().len() < graph.num_vertices() as usize);
    assert_eq!(decomposition.bags().last().unwrap().vertices().len(), 80);
}

#[test]
fn a_sampled_order_stopped_after_an_elimination_still_bags_that_vertex() {
    // Sampled min-fill removes a vertex, then repairs the fill scores of its
    // neighbours, and reads the deadline while it does. A run stopped there has
    // already taken the vertex out of the graph, so the engine's residual bag
    // does not cover it and the bag it was about to record is the only place it
    // would appear.
    let graph = chorded_ring(2_000);
    let weights = vec![1u32; graph.num_vertices() as usize];
    let decomposition =
        complete_at_immediate_deadline(&graph, Order::MinFillSampled { weights: &weights });

    decomposition.validate(&graph).unwrap();
}
