use std::collections::BTreeSet;

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

/// Gadgets the five rules fire on, appended to `edges` from index `base`, and
/// the first index past them: an isolate, a pendant path, a square, a K4 that
/// raises the treewidth bound to three, and a K4 missing one edge with a twin
/// of one of its vertices. The last gadget has no simplicial vertex, so the
/// almost-simplicial rule is what clears it.
fn reduction_gadgets(base: u32, edges: &mut Vec<(u32, u32)>) -> u32 {
    // The isolate.
    let mut next = base + 1;

    // A pendant path onto the square, peeled by the twig rule before the
    // square's vertices reach degree two.
    let path = next;
    next += 2;
    let square = next;
    next += 4;
    edges.extend([(path, path + 1), (path + 1, square)]);
    edges.extend([
        (square, square + 1),
        (square + 1, square + 2),
        (square + 2, square + 3),
        (square, square + 3),
    ]);

    let k4 = next;
    next += 4;
    for a in 0..4 {
        for b in a + 1..4 {
            edges.push((k4 + a, k4 + b));
        }
    }

    let almost = next;
    next += 5;
    edges.extend([
        (almost, almost + 1),
        (almost, almost + 2),
        (almost, almost + 3),
        (almost + 1, almost + 2),
        (almost + 1, almost + 3),
        (almost + 4, almost + 1),
        (almost + 4, almost + 2),
        (almost + 4, almost + 3),
    ]);

    next
}

/// A chorded ring, which no rule fires on, so it is what the gadgets beside it
/// leave as the residual — and it holds enough vertices to keep the graph out
/// of bitset mode.
fn chorded_ring(vertices: u32, edges: &mut Vec<(u32, u32)>) {
    for vertex in 0..vertices {
        edges.push((vertex, (vertex + 1) % vertices));
        edges.push((vertex, (vertex + 7) % vertices));
        edges.push((vertex, (vertex + 53) % vertices));
    }
}

/// Every recorded bag is the eliminated vertex followed by the neighbours it
/// had at that step, and the residual holds what eliminating those vertices
/// with fill leaves. The check runs on neighbourhoods rather than adjacency
/// rows, so it reads the same in both representations.
fn assert_bags_and_residual_match_fill_eliminations(n: u32, edges: &[(u32, u32)]) {
    let reduced = preprocess(EliminationGraph::from_edges(n, edges), None);

    let mut neighbourhood: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); n as usize];
    for &(u, v) in edges {
        neighbourhood[u as usize].insert(v);
        neighbourhood[v as usize].insert(u);
    }
    let mut live = vec![true; n as usize];

    let steps = reduced.prefix.bags.iter().zip(&reduced.prefix.rank_pairs);
    for (index, (bag, &(vertex, step))) in steps.enumerate() {
        assert_eq!(step, index, "bag {index} is not the step it is ranked at");
        assert_eq!(bag[0], vertex, "bag {index} does not start with its vertex");
        let recorded: BTreeSet<u32> = bag[1..].iter().copied().collect();
        assert_eq!(
            recorded.len(),
            bag.len() - 1,
            "bag {index} lists a neighbour twice"
        );
        assert_eq!(
            recorded, neighbourhood[vertex as usize],
            "bag {index} is not {vertex} and its neighbours"
        );

        // Eliminating with fill: the neighbourhood becomes a clique, then the
        // vertex leaves it.
        let nbrs: Vec<u32> = neighbourhood[vertex as usize].iter().copied().collect();
        for (position, &u) in nbrs.iter().enumerate() {
            for &w in &nbrs[position + 1..] {
                neighbourhood[u as usize].insert(w);
                neighbourhood[w as usize].insert(u);
            }
        }
        for &u in &nbrs {
            neighbourhood[u as usize].remove(&vertex);
        }
        neighbourhood[vertex as usize].clear();
        live[vertex as usize] = false;
    }

    let mut residual_ends = 0usize;
    for vertex in 0..n {
        assert_eq!(
            reduced.graph.active[vertex as usize], live[vertex as usize],
            "vertex {vertex} is on the wrong side of the residual"
        );
        if !live[vertex as usize] {
            continue;
        }
        let mut have = reduced.graph.live_neighbours(vertex);
        have.sort_unstable();
        let want: Vec<u32> = neighbourhood[vertex as usize].iter().copied().collect();
        assert_eq!(have, want, "residual neighbours of {vertex}");
        residual_ends += want.len();
    }
    assert_eq!(reduced.graph.num_edges, residual_ends / 2);
    assert_eq!(
        reduced.graph.num_active,
        live.iter().filter(|&&alive| alive).count()
    );
}

#[test]
fn recorded_bags_and_the_residual_match_eliminating_with_fill() {
    // All five rules on a graph dense enough for the bitset path.
    let mut dense = Vec::new();
    let dense_n = reduction_gadgets(0, &mut dense);
    assert_bags_and_residual_match_fill_eliminations(dense_n, &dense);

    // The same rules on a graph too sparse for it, so short adjacency rows
    // answer, and with a residual left over.
    let mut sparse = Vec::new();
    let ring = 2_000u32;
    chorded_ring(ring, &mut sparse);
    let sparse_n = reduction_gadgets(ring, &mut sparse);
    assert_bags_and_residual_match_fill_eliminations(sparse_n, &sparse);

    // A clique whose rows are long enough to carry membership maps, in a graph
    // past the bitset vertex bound: every vertex is simplicial, so this is the
    // rule that removes a clique neighbourhood, on indexed rows.
    let clique = 260u32;
    let mut indexed = complete_graph(clique);
    let indexed_n = reduction_gadgets(clique, &mut indexed);
    assert!(indexed_n < 20_000, "past BITSET_THRESH, so the rows answer");
    assert_bags_and_residual_match_fill_eliminations(20_000, &indexed);
}
