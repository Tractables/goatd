use crate::elimination::graph::*;

#[test]
fn from_edges_builds_undirected_adjacency() {
    let g = EliminationGraph::from_edges(3, &[(0, 1), (1, 2)]);
    assert_eq!(g.degree(0), 1);
    assert_eq!(g.degree(1), 2);
    assert_eq!(g.degree(2), 1);
    assert!(g.contains_edge(0, 1) && g.contains_edge(1, 0));
}

#[test]
fn eliminate_fills_clique_and_deactivates() {
    let mut g = EliminationGraph::from_edges(3, &[(0, 1), (1, 2)]);
    let bag = g.eliminate(1);
    assert_eq!(bag.len(), 2);
    assert!(g.contains_edge(0, 2));
    assert!(!g.active[1]);
    assert_eq!(g.num_active, 2);
}

#[test]
fn eliminating_records_each_new_fill_edge_once() {
    for n in [4, 200] {
        let mut graph = EliminationGraph::from_edges(n, &[(0, 1), (0, 2), (0, 3), (1, 2)]);
        let neighbours = graph.live_neighbours(0);
        let mut fill_edges = Vec::new();

        graph.eliminate_with_nbrs_record_fill(0, &neighbours, &mut fill_edges);
        fill_edges.sort_unstable();

        assert_eq!(fill_edges, [(1, 3), (2, 3)]);
    }
}

#[test]
fn simplicial_detection() {
    let g = EliminationGraph::from_edges(3, &[(0, 1), (0, 2), (1, 2)]);
    assert!(g.is_simplicial(0));
    let g = EliminationGraph::from_edges(3, &[(0, 1), (1, 2)]);
    assert!(!g.is_simplicial(1));
    assert!(g.is_simplicial(0));
}

#[test]
fn from_edges_dedups_repeated_inputs() {
    let g = EliminationGraph::from_edges(2, &[(0, 1), (1, 0), (0, 1)]);
    assert_eq!(g.degree(0), 1);
    assert_eq!(g.degree(1), 1);
}

#[test]
fn edge_query_agrees_with_the_adjacency_lists() {
    let g = EliminationGraph::from_edges(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (0, 4)]);
    assert!(g.bitset_words > 0);
    for u in 0u32..5 {
        for v in 0u32..5 {
            let adj_has = g.adj[u as usize].contains(&v);
            let bs_has = g.contains_edge(u, v);
            assert_eq!(adj_has, bs_has, "u={u} v={v}");
        }
    }
}

#[test]
fn fill_count_is_the_edges_missing_among_the_neighbours() {
    let g = EliminationGraph::from_edges(3, &[(0, 1), (1, 2)]);
    assert_eq!(g.fill_count_of_bs(1), 1);
    assert_eq!(g.fill_count_of_bs(0), 0);
    let g2 = EliminationGraph::from_edges(3, &[(0, 1), (0, 2), (1, 2)]);
    assert_eq!(g2.fill_count_of_bs(0), 0);
    assert_eq!(g2.fill_count_of_bs(1), 0);
}

#[test]
fn bitset_difference_counts_only_left_neighbours() {
    let graph = EliminationGraph::from_edges(4, &[(0, 1), (0, 2), (0, 3), (1, 2)]);
    assert!(graph.bitset_words > 0);
    assert_eq!(graph.bitset_difference_count(0, 1), 2);
    assert_eq!(graph.bitset_difference_count(1, 0), 1);
}

#[test]
fn bitset_intersection_count_handles_four_word_chunks_and_tail() {
    let left = [
        0xffff,
        u64::MAX,
        0,
        0xf0f0,
        1,
        1 << 63,
        0xaaaa,
        0x0f0f,
        0b1100,
    ];
    let right = [
        0x00ff,
        0x5555_5555_5555_5555,
        u64::MAX,
        0x3333,
        1,
        1 << 63,
        0x5555,
        0x00ff,
        0b1010,
    ];

    assert_eq!(intersection_popcount(&left, &right), 51);
}

#[test]
fn dense_fill_count_handles_a_partial_final_word() {
    let mut edges = Vec::new();
    for vertex in 1..=130 {
        edges.push((0, vertex));
    }
    for vertex in 1..130 {
        edges.push((vertex, vertex + 1));
    }
    for left in 131..171 {
        for right in (left + 1)..171 {
            edges.push((left, right));
        }
    }

    let graph = EliminationGraph::from_edges(300, &edges);
    assert_eq!(graph.bitset_words, 5);
    assert_eq!(graph.fill_count_of_bs(0), 8_256);
    assert_eq!(
        graph.fill_count_of_bs(0),
        graph.fill_count_of_bs_portable(0)
    );
}

#[test]
fn promote_bitset_from_sparse_graph() {
    let n = 200u32;
    let edges: Vec<(u32, u32)> = (0..n - 1).map(|v| (v, v + 1)).collect();
    let mut g = EliminationGraph::from_edges(n, &edges);
    assert_eq!(g.bitset_words, 0, "sparse path expected");
    assert!(!g.should_promote_bitset(), "sparse density below threshold");
    g.promote_bitset();
    assert!(g.bitset_words > 0, "bitset populated after promotion");
    assert_eq!(g.degree(0), 1);
    assert_eq!(g.degree(100), 2);
    for v in 0..n - 1 {
        assert!(g.contains_edge(v, v + 1));
        assert!(g.contains_edge(v + 1, v));
    }
    assert!(!g.contains_edge(0, 5));
}

#[test]
fn should_promote_bitset_triggers_when_dense() {
    // Near-complete graph on 128 vertices, dense enough to cross the
    // promotion threshold.
    let n = 128u32;
    let mut edges = Vec::new();
    for u in 0..n {
        for v in (u + 1)..n {
            edges.push((u, v));
        }
    }
    // Construct sparse (first 10 edges only) then add_edge the rest, so the
    // graph starts adj-only regardless of what from_edges' own threshold does.
    let mut g = EliminationGraph::from_edges(n, &edges[..10]);
    for &(u, v) in &edges[10..] {
        g.add_edge(u, v);
    }
    // n=128 ≤ BITSET_THRESH so from_edges may already have enabled bitset;
    // if not, promotion should fire immediately.
    if g.bitset_words == 0 {
        assert!(g.should_promote_bitset());
    }
}

#[test]
fn eliminating_a_vertex_replaces_its_edges_with_the_fill_edge() {
    let mut g = EliminationGraph::from_edges(3, &[(0, 1), (1, 2)]);
    assert!(g.bitset_words > 0, "should use bitset for n=3");
    g.eliminate(1);
    assert!(g.contains_edge(0, 2));
    assert!(!g.active[1]);
    assert_eq!(g.num_active, 2);
    assert_eq!(g.num_edges, 1);
}

#[test]
fn cached_bitset_degrees_follow_fill_and_removal() {
    let mut graph = EliminationGraph::from_edges(4, &[(0, 1), (0, 2), (0, 3), (1, 2)]);
    assert!(graph.bitset_words > 0);
    graph.add_edge(2, 3);
    assert_bitset_degrees(&graph);

    graph.eliminate(0);
    assert_bitset_degrees(&graph);

    let neighbours = graph.live_neighbours(1);
    graph.remove_without_fill_nbrs(1, &neighbours);
    assert_bitset_degrees(&graph);
}

fn assert_bitset_degrees(graph: &EliminationGraph) {
    for vertex in 0..graph.len() {
        let start = vertex * graph.bitset_words;
        let actual = graph.bitset[start..start + graph.bitset_words]
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum::<usize>();
        assert_eq!(graph.degree(vertex as u32), actual, "vertex {vertex}");
    }
}

/// A star whose centre is well past `ROW_INDEX_THRESH`, on a vertex count
/// above `BITSET_THRESH` so the graph stays in sparse mode.
fn hub_graph(spokes: u32) -> EliminationGraph {
    let edges: Vec<(u32, u32)> = (1..=spokes).map(|spoke| (0, spoke)).collect();
    EliminationGraph::from_edges(20_000, &edges)
}

#[test]
fn a_long_row_is_indexed_and_a_short_one_is_not() {
    let graph = hub_graph(ROW_INDEX_THRESH as u32 + 100);
    assert!(graph.row_is_indexed(0));
    assert!(!graph.row_is_indexed(1));

    let graph = hub_graph(ROW_INDEX_THRESH as u32 - 100);
    assert!(!graph.row_is_indexed(0));
}

#[test]
fn an_indexed_row_dedups_repeated_input_edges() {
    let spokes = ROW_INDEX_THRESH as u32 + 100;
    let mut edges: Vec<(u32, u32)> = (1..=spokes).map(|spoke| (0, spoke)).collect();
    edges.extend((1..=spokes).map(|spoke| (spoke, 0)));
    edges.extend((1..=spokes).map(|spoke| (0, spoke)));
    let graph = EliminationGraph::from_edges(20_000, &edges);

    assert!(graph.row_is_indexed(0));
    assert_eq!(graph.degree(0), spokes as usize);
    assert_eq!(graph.num_edges, spokes as usize);
    for spoke in 1..=spokes {
        assert_eq!(graph.degree(spoke), 1);
        assert!(graph.contains_edge(0, spoke) && graph.contains_edge(spoke, 0));
    }
    assert!(!graph.contains_edge(0, spokes + 1));
}

#[test]
fn eliminating_beside_an_indexed_row_leaves_the_row_a_scan_would() {
    let spokes = ROW_INDEX_THRESH as u32 + 100;
    let mut graph = hub_graph(spokes);
    // Give spoke 1 a second neighbour, so eliminating it fills an edge from
    // the hub to that neighbour.
    graph.add_edge(1, spokes + 1);
    assert!(graph.row_is_indexed(0));

    let neighbours = graph.live_neighbours(1);
    graph.eliminate_with_nbrs(1, &neighbours);

    // The scan-and-`swap_remove` this replaces takes spoke 1 out of position 0
    // by moving the last spoke into it, then appends the fill neighbour.
    let mut expected: Vec<u32> = vec![spokes];
    expected.extend(2..spokes);
    expected.push(spokes + 1);
    assert_eq!(graph.adj[0], expected);
    assert!(graph.contains_edge(0, spokes + 1));
    assert!(!graph.contains_edge(0, 1));
    assert!(graph.is_simplicial(spokes + 1));
}

#[test]
fn removing_the_hub_clears_its_row_and_its_index() {
    let spokes = ROW_INDEX_THRESH as u32 + 100;
    let mut graph = hub_graph(spokes);
    let neighbours = graph.live_neighbours(0);

    graph.remove_without_fill_nbrs(0, &neighbours);

    assert!(!graph.row_is_indexed(0));
    assert_eq!(graph.degree(0), 0);
    assert_eq!(graph.num_edges, 0);
    for spoke in 1..=spokes {
        assert_eq!(graph.degree(spoke), 0);
    }
}

/// `canonical` with every edge immediately repeated twice more, one of the
/// copies reversed, so the general path has to deduplicate a list whose first
/// occurrences are still in sorted order and self-loops have to be dropped.
fn with_duplicates(canonical: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut edges = Vec::new();
    for &edge in canonical {
        edges.push(edge);
        edges.push((edge.1, edge.0));
        edges.push((edge.0, edge.0));
        edges.push(edge);
    }
    edges
}

/// The ring on `0..n`, in canonical order.
fn ring(n: u32) -> Vec<(u32, u32)> {
    let mut edges: Vec<(u32, u32)> = (0..n)
        .map(|v| {
            let w = (v + 1) % n;
            if v < w { (v, w) } else { (w, v) }
        })
        .collect();
    edges.sort_unstable();
    edges
}

/// The ring on `0..n` plus a hub `n` adjacent to every ring vertex, in
/// canonical order. The hub's row is long enough to carry a membership map.
fn wheel(n: u32) -> Vec<(u32, u32)> {
    let mut edges = ring(n);
    edges.extend((0..n).map(|v| (v, n)));
    edges.sort_unstable();
    edges
}

/// The complete graph on `0..n`, in canonical order, which `from_edges` keeps
/// as a bitset.
fn clique(n: u32) -> Vec<(u32, u32)> {
    let mut edges = Vec::new();
    for u in 0..n {
        for v in u + 1..n {
            edges.push((u, v));
        }
    }
    edges
}

#[test]
fn the_canonical_build_agrees_with_the_deduplicating_one() {
    let cases = [
        ("ring 8", 8u32, ring(8)),
        ("ring 600", 600, ring(600)),
        ("wheel 600", 601, wheel(600)),
        ("clique 40", 40, clique(40)),
    ];
    for (name, n, canonical) in cases {
        let repeated = with_duplicates(&canonical);
        assert_eq!(crate::graph::canonical_edges(repeated.clone()), canonical);

        let from_canonical = EliminationGraph::from_edges(n, &canonical);
        let from_repeated = EliminationGraph::from_edges(n, &repeated);

        assert_eq!(from_canonical.num_edges, canonical.len(), "{name}");
        assert_eq!(from_canonical.num_edges, from_repeated.num_edges, "{name}");
        assert_eq!(
            from_canonical.bitset_words, from_repeated.bitset_words,
            "{name}"
        );
        assert_eq!(from_canonical.bitset, from_repeated.bitset, "{name}");
        for v in 0..n {
            assert_eq!(
                from_canonical.adj[v as usize], from_repeated.adj[v as usize],
                "{name}, row {v}"
            );
            assert_eq!(
                from_canonical.row_is_indexed(v),
                from_repeated.row_is_indexed(v),
                "{name}, row {v}"
            );
            assert_eq!(
                from_canonical.degree(v),
                from_repeated.degree(v),
                "{name}, row {v}"
            );
        }
    }
    assert!(EliminationGraph::from_edges(601, &wheel(600)).row_is_indexed(600));
    assert!(EliminationGraph::from_edges(40, &clique(40)).bitset_words > 0);
}

#[test]
fn the_canonical_build_orders_a_hub_row_the_way_the_edge_list_does() {
    // A hub long enough to carry a membership map, so the fast path has to
    // build one and to leave the row in the order the sorted list implies:
    // the neighbours below the hub first, then those above it.
    let hub = 300u32;
    let n = 601u32;
    let mut edges: Vec<(u32, u32)> = (0..n)
        .filter(|&v| v != hub)
        .map(|v| (v.min(hub), v.max(hub)))
        .collect();
    edges.sort_unstable();

    let graph = EliminationGraph::from_edges(n, &edges);

    assert!(graph.row_is_indexed(hub));
    let expected: Vec<u32> = (0..hub).chain(hub + 1..n).collect();
    assert_eq!(graph.adj[hub as usize], expected);
}

#[test]
fn the_bitset_almost_simplicial_test_agrees_with_the_pairwise_scan() {
    // K5 with (1, 2) removed: vertex 0's neighbourhood misses exactly that
    // edge. Removing a second edge leaves two missing and no answer.
    let mut edges = Vec::new();
    for u in 0u32..5 {
        for v in u + 1..5 {
            edges.push((u, v));
        }
    }
    let one_missing: Vec<(u32, u32)> = edges.iter().copied().filter(|&e| e != (1, 2)).collect();
    let graph = EliminationGraph::from_edges(5, &one_missing);
    assert!(graph.bitset_words > 0);
    assert_eq!(graph.almost_simplicial_nonedge(0), Some((1, 2)));

    let two_missing: Vec<(u32, u32)> = one_missing
        .iter()
        .copied()
        .filter(|&e| e != (3, 4))
        .collect();
    let graph = EliminationGraph::from_edges(5, &two_missing);
    assert_eq!(graph.almost_simplicial_nonedge(0), None);

    let complete = EliminationGraph::from_edges(5, &edges);
    assert_eq!(complete.almost_simplicial_nonedge(0), None);
}
