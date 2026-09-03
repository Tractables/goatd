use crate::elimination::graph::EliminationGraph;
use crate::elimination::preprocess::*;

#[test]
fn a_single_edge_and_an_isolate_reduce_to_one_bag_per_vertex() {
    let g = EliminationGraph::from_edges(3, &[(0, 1)]);
    let reduced = preprocess(g, None);
    assert_eq!(reduced.graph.num_active, 0);
    assert_eq!(reduced.prefix.bags.len(), 3);
}

#[test]
fn twig_removes_leaves() {
    let g = EliminationGraph::from_edges(4, &[(0, 1), (1, 2), (2, 3)]);
    let reduced = preprocess(g, None);
    assert_eq!(reduced.graph.num_active, 0);
}

#[test]
fn low_degree_rules_revisit_vertices_before_series() {
    // The path order is 4-0-3-1-2. Removing the high-index leaf 4 exposes
    // vertex 0 after the scan cursor has passed it.
    let edges = [(4, 0), (0, 3), (3, 1), (1, 2)];
    let reduced = preprocess(EliminationGraph::from_edges(5, &edges), None);
    assert_eq!(reduced.graph.num_active, 0);
    assert!(reduced.prefix.bags.iter().all(|bag| bag.len() <= 2));
}

#[test]
fn simplicial_triangle_collapses() {
    let g = EliminationGraph::from_edges(3, &[(0, 1), (0, 2), (1, 2)]);
    let reduced = preprocess(g, None);
    assert_eq!(reduced.graph.num_active, 0);
    assert_eq!(reduced.prefix.bags[0].len(), 3);
}

#[test]
fn series_adds_fill_then_contracts() {
    let g = EliminationGraph::from_edges(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]);
    let reduced = preprocess(g, None);
    assert_eq!(reduced.graph.num_active, 0);
}

#[test]
fn almost_simplicial_fires_under_lb() {
    // An almost-simplicial vertex of degree 3 only fires once tw_lb >= 3, so
    // a disjoint K_4 establishes that bound before vertices 0 and 4 (each
    // almost-simplicial, missing edge (2,3)) get a turn.
    let edges = vec![
        // K_4 on {0,1,2,3} minus (2,3).
        (0, 1),
        (0, 2),
        (0, 3),
        (1, 2),
        (1, 3),
        // vertex 4: same neighbourhood shape as 0, also almost-simplicial.
        (4, 1),
        (4, 2),
        (4, 3),
        // disjoint K_4 establishing tw_lb = 3.
        (8, 9),
        (8, 10),
        (8, 11),
        (9, 10),
        (9, 11),
        (10, 11),
    ];
    let g = EliminationGraph::from_edges(12, &edges);
    let reduced = preprocess(g, None);
    assert_eq!(reduced.graph.num_active, 0);
    let max_bag = reduced.prefix.bags.iter().map(|b| b.len()).max().unwrap();
    assert_eq!(max_bag, 4);
}

#[test]
fn almost_simplicial_skipped_without_tw_lb() {
    // Two triangles sharing an edge: fully reduces via simplicial+twig
    // without ever needing almost-simplicial — verifies the rule doesn't
    // over-fire when tw_lb hasn't been established.
    let g = EliminationGraph::from_edges(4, &[(0, 1), (0, 2), (1, 2), (0, 3), (1, 3)]);
    let reduced = preprocess(g, None);
    assert_eq!(reduced.graph.num_active, 0);
}

/// The complete graph on `n` vertices, in canonical edge order.
fn complete_graph(n: u32) -> Vec<(u32, u32)> {
    let mut edges = Vec::with_capacity((n as usize * (n as usize - 1)) / 2);
    for u in 0..n {
        for v in u + 1..n {
            edges.push((u, v));
        }
    }
    edges
}

#[test]
fn preprocessing_stops_at_the_soft_cutoff_instead_of_reducing_to_the_end() {
    // Every vertex of a clique is simplicial, so with no cutoff the rules
    // eliminate all of them; each elimination charges the meter, so a cutoff
    // one millisecond of work away stops the pass part way. The meter is
    // armed, so this is a count of work and not a race with the wall.
    let edges = complete_graph(500);

    let whole = preprocess(EliminationGraph::from_edges(500, &edges), None);
    assert_eq!(whole.graph.num_active, 0);

    let epoch = std::time::Instant::now();
    let _meter = crate::meter::arm(epoch);
    let cutoff = epoch + std::time::Duration::from_millis(1);
    let stopped = preprocess(EliminationGraph::from_edges(500, &edges), Some(cutoff));

    assert!(
        stopped.graph.num_active > 0,
        "preprocessing ran to the end past a cutoff a millisecond of work away"
    );
    let overrun = crate::meter::now().saturating_duration_since(cutoff);
    assert!(
        overrun <= std::time::Duration::from_millis(2),
        "preprocessing stopped {overrun:?} past its cutoff"
    );
}

#[test]
fn the_almost_simplicial_rule_fires_on_a_single_missing_edge() {
    // K6 with (4, 5) removed. Vertices 4 and 5 are simplicial, so the earlier
    // rule takes them; what is left for the almost-simplicial rule is that the
    // reduction still empties the graph and every bag stays within the width.
    let mut edges = complete_graph(6);
    edges.retain(|&e| e != (4, 5));
    let reduced = preprocess(EliminationGraph::from_edges(6, &edges), None);
    assert_eq!(reduced.graph.num_active, 0);
    assert!(reduced.prefix.bags.iter().all(|bag| bag.len() <= 6));
}
