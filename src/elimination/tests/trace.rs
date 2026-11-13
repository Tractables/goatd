//! Traced eliminations: the order and neighbours a run reports, against an
//! elimination recomputed from scratch at every step.

use std::collections::BTreeSet;

use crate::elimination::{Order, RunConfig, Trace, TraceOutcome, trace};
use crate::{Error, Graph};

/// Edges of a graph on `n` vertices, each pair joined with odds
/// `per_mille / 1000`, from a fixed stream.
fn random_edges(n: u32, per_mille: u64, state: &mut u64) -> Vec<(u32, u32)> {
    let mut edges = Vec::new();
    for u in 0..n {
        for v in u + 1..n {
            *state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            if (*state >> 33) % 1000 < per_mille {
                edges.push((u, v));
            }
        }
    }
    edges
}

/// The elimination that takes, at every step, the vertex of least weighted
/// fill, then degree, then rank, then number, each score recounted.
fn reference(n: u32, edges: &[(u32, u32)], ranks: &[u32], weights: &[u64]) -> Trace {
    let mut adjacent = vec![BTreeSet::new(); n as usize];
    for &(u, v) in edges {
        adjacent[u as usize].insert(v);
        adjacent[v as usize].insert(u);
    }
    let mut live: BTreeSet<u32> = (0..n).collect();
    let mut order = Vec::new();
    let mut neighbours = Vec::new();
    while !live.is_empty() {
        let score = |v: u32| {
            let around: Vec<u32> = adjacent[v as usize].iter().copied().collect();
            let mut fill = 0;
            for (i, &a) in around.iter().enumerate() {
                for &b in &around[i + 1..] {
                    if !adjacent[a as usize].contains(&b) {
                        fill += weights[a as usize] * weights[b as usize];
                    }
                }
            }
            (fill, around.len(), ranks[v as usize], v)
        };
        let v = live.iter().copied().min_by_key(|&v| score(v)).unwrap();
        let around: Vec<u32> = adjacent[v as usize].iter().copied().collect();
        for &a in &around {
            adjacent[a as usize].remove(&v);
            for &b in &around {
                if a != b {
                    adjacent[a as usize].insert(b);
                }
            }
        }
        live.remove(&v);
        order.push(v);
        neighbours.push(around);
    }
    Trace { order, neighbours }
}

fn completed(outcome: TraceOutcome) -> Trace {
    match outcome {
        TraceOutcome::Completed(trace) => sorted(trace),
        other => panic!("the trace did not complete: {other:?}"),
    }
}

fn sorted(mut trace: Trace) -> Trace {
    for around in &mut trace.neighbours {
        around.sort_unstable();
    }
    trace
}

/// A vertex count, edges, ranks and weights.
type Case = (u32, Vec<(u32, u32)>, Vec<u32>, Vec<u64>);

/// Graphs sparse and dense, small and past the size at which the
/// elimination keeps its graph as rows of bits, with ranks and weights.
fn cases() -> Vec<Case> {
    let mut state = 7;
    let mut cases = Vec::new();
    for &(n, per_mille) in &[
        (1, 0),
        (6, 400),
        (12, 150),
        (20, 600),
        (30, 100),
        (40, 300),
        (150, 15),
        (300, 8),
    ] {
        for round in 0..4u32 {
            let edges = random_edges(n, per_mille, &mut state);
            let ranks: Vec<u32> = (0..n).map(|v| (v * 7 + round) % n.max(1)).collect();
            let weights: Vec<u64> = (0..n)
                .map(|v| 1 + u64::from((v * 13 + round) % 5))
                .collect();
            cases.push((n, edges, ranks, weights));
        }
    }
    cases
}

#[test]
fn ranked_min_fill_traces_the_least_fill_degree_and_rank_at_every_step() {
    for (n, edges, ranks, _) in cases() {
        let graph = Graph::new(n, edges.clone());
        let order = Order::MinFillRanked { ranks: &ranks };
        let traced = completed(trace(&graph, order, 0, RunConfig::default()).unwrap());
        assert_eq!(traced, reference(n, &edges, &ranks, &vec![1; n as usize]));
    }
}

#[test]
fn weighted_min_fill_traces_the_least_weighted_fill_at_every_step() {
    for (n, edges, ranks, weights) in cases() {
        let graph = Graph::new(n, edges.clone());
        let order = Order::MinFillWeighted {
            ranks: &ranks,
            weights: &weights,
        };
        let traced = completed(trace(&graph, order, 0, RunConfig::default()).unwrap());
        assert_eq!(traced, reference(n, &edges, &ranks, &weights));
    }
}

#[test]
fn weighted_min_fill_at_weight_one_is_ranked_min_fill() {
    for (n, edges, ranks, _) in cases() {
        let graph = Graph::new(n, edges);
        let ones = vec![1; n as usize];
        let weighted = Order::MinFillWeighted {
            ranks: &ranks,
            weights: &ones,
        };
        let ranked = Order::MinFillRanked { ranks: &ranks };
        assert_eq!(
            trace(&graph, weighted, 0, RunConfig::default()).unwrap(),
            trace(&graph, ranked, 0, RunConfig::default()).unwrap()
        );
    }
}

#[test]
fn a_weighted_min_fill_decomposition_matches_its_trace() {
    // Through `decompose`, which reduces the graph and splits it into its
    // components, the weights still reach each component's elimination.
    for (n, edges, ranks, weights) in cases() {
        let graph = Graph::new(n, edges);
        let order = Order::MinFillWeighted {
            ranks: &ranks,
            weights: &weights,
        };
        let td = crate::elimination::decompose(&graph, order, 0, None).unwrap();
        td.validate(&graph).unwrap();
    }
}

#[test]
fn ranked_traces_eliminate_in_the_callers_order() {
    for (n, edges, ranks, _) in cases() {
        let graph = Graph::new(n, edges.clone());
        let traced = completed(
            trace(
                &graph,
                Order::Ranked { ranks: &ranks },
                0,
                RunConfig::default(),
            )
            .unwrap(),
        );
        let mut expected: Vec<u32> = (0..n).collect();
        expected.sort_by_key(|&v| (ranks[v as usize], v));
        assert_eq!(traced.order(), expected.as_slice());
        assert_eq!(traced.neighbours().len(), n as usize);
    }
}

#[test]
fn a_trace_decomposition_is_numbered_from_the_last_vertex() {
    for (n, edges, ranks, _) in cases() {
        let graph = Graph::new(n, edges);
        let traced = completed(
            trace(
                &graph,
                Order::MinFillRanked { ranks: &ranks },
                0,
                RunConfig::default(),
            )
            .unwrap(),
        );
        let td = traced.decomposition(&graph).unwrap();
        td.validate(&graph).unwrap();
        let bags = td.bags();
        assert_eq!(bags.len(), n as usize);
        // Bag `i` is the bag of the vertex eliminated `i`-th from the last,
        // and its parent, the bag of its first neighbour to go, is numbered
        // below it.
        for (i, (&v, around)) in traced
            .order()
            .iter()
            .zip(traced.neighbours())
            .rev()
            .enumerate()
        {
            let mut bag: Vec<u32> = around.iter().copied().chain([v]).collect();
            bag.sort_unstable();
            assert_eq!(bags[i].vertices(), bag.as_slice());
            let below = td.adjacency()[i].iter().filter(|&&j| j < i).count();
            assert_eq!(below, usize::from(!around.is_empty()));
        }
    }
}

#[test]
fn isolated_vertices_go_first_whatever_their_component() {
    // A path 0-1-2 and two isolated vertices: unsplit, min-fill takes the
    // vertices without neighbours before any other.
    let graph = Graph::new(5, vec![(0, 1), (1, 2)]);
    let ranks = [0, 1, 2, 3, 4];
    let traced = completed(
        trace(
            &graph,
            Order::MinFillRanked { ranks: &ranks },
            0,
            RunConfig::default(),
        )
        .unwrap(),
    );
    assert_eq!(traced.order(), &[3, 4, 0, 1, 2]);
}

#[test]
fn a_clique_left_at_the_end_goes_by_rank() {
    // Two triangles on a shared edge: once a tip goes, the rest is a
    // triangle, which goes by rank.
    let graph = Graph::new(4, vec![(0, 1), (0, 2), (1, 2), (1, 3), (2, 3)]);
    let ranks = [3, 2, 0, 1];
    let traced = completed(
        trace(
            &graph,
            Order::MinFillRanked { ranks: &ranks },
            0,
            RunConfig::default(),
        )
        .unwrap(),
    );
    assert_eq!(traced.order(), &[3, 2, 1, 0]);
}

#[test]
fn a_trace_past_its_width_bound_aborts() {
    let graph = Graph::new(4, vec![(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]);
    let ranks = [0, 1, 2, 3];
    let order = Order::Ranked { ranks: &ranks };
    let narrow = RunConfig::default().with_width_bound(Some(2));
    assert_eq!(
        trace(&graph, order, 0, narrow).unwrap(),
        TraceOutcome::WidthAborted
    );
    let wide = RunConfig::default().with_width_bound(Some(3));
    assert!(matches!(
        trace(&graph, order, 0, wide).unwrap(),
        TraceOutcome::Completed(_)
    ));
}

#[test]
fn weighted_min_fill_refuses_weights_that_do_not_fit() {
    let graph = Graph::new(2, vec![(0, 1)]);
    let ranks = [0, 1];
    for weights in [&[1, 0][..], &[1][..], &[1 << 31, 1 << 31][..]] {
        let order = Order::MinFillWeighted {
            ranks: &ranks,
            weights,
        };
        assert!(matches!(
            trace(&graph, order, 0, RunConfig::default()),
            Err(Error::InvalidInput(_))
        ));
    }
}
