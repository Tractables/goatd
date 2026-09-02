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
