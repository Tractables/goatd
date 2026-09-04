//! The two cardinality searches on graphs whose treewidth is known, the
//! perfect elimination ordering the plain search gives a chordal graph, and
//! both orderings' determinism.

use std::time::{Duration, Instant};

use crate::elimination::minimal_triangulation::{Reach, Ties, cardinality_search};
use crate::elimination::{Order, decompose};
use crate::{Graph, TreeDecomposition};

/// A cycle on `n` vertices: treewidth 2.
fn cycle(n: u32) -> Graph {
    Graph::new(n, (0..n).map(|vertex| (vertex, (vertex + 1) % n)))
}

/// The `rows` by `columns` grid: treewidth `min(rows, columns)`.
fn grid(rows: u32, columns: u32) -> Graph {
    let mut edges = Vec::new();
    for row in 0..rows {
        for column in 0..columns {
            let vertex = row * columns + column;
            if column + 1 < columns {
                edges.push((vertex, vertex + 1));
            }
            if row + 1 < rows {
                edges.push((vertex, vertex + columns));
            }
        }
    }
    Graph::new(rows * columns, edges)
}

/// The complete bipartite graph on `left` and `right` vertices: treewidth
/// `min(left, right)`.
fn complete_bipartite(left: u32, right: u32) -> Graph {
    let edges = (0..left).flat_map(|a| (0..right).map(move |b| (a, left + b)));
    Graph::new(left + right, edges)
}

/// A `k`-tree on `n` vertices: the first `k + 1` form a clique and every later
/// vertex joins the `k` before it. Chordal, of treewidth `k`.
fn k_tree(n: u32, k: u32) -> Graph {
    let mut edges = Vec::new();
    for vertex in 1..n {
        for earlier in vertex.saturating_sub(k)..vertex {
            edges.push((earlier, vertex));
        }
    }
    Graph::new(n, edges)
}

fn adjacency_of(graph: &Graph) -> Vec<Vec<u32>> {
    let mut adjacency = vec![Vec::new(); graph.num_vertices as usize];
    for &(a, b) in &graph.edges {
        adjacency[a as usize].push(b);
        adjacency[b as usize].push(a);
    }
    adjacency
}

fn decomposed_by(graph: &Graph, order: Order<'_>) -> TreeDecomposition {
    let td = decompose(graph, order, 0, None).expect("a deterministic order takes no weights");
    td.validate(graph)
        .expect("the order produces a decomposition of its graph");
    td
}

fn minimal_triangulation(graph: &Graph) -> TreeDecomposition {
    decomposed_by(graph, Order::MinimalTriangulation)
}

fn maximum_cardinality(graph: &Graph) -> TreeDecomposition {
    decomposed_by(graph, Order::MaximumCardinality)
}

#[test]
fn cycle_is_triangulated_to_width_two() {
    // Every minimal triangulation of a cycle is a triangulation of the polygon
    // it bounds, whose maximal cliques are triangles.
    for n in [4, 7, 10, 25] {
        let graph = cycle(n);
        assert_eq!(minimal_triangulation(&graph).treewidth(), 2, "cycle of {n}");
    }
}

#[test]
fn grids_are_triangulated_near_their_treewidth() {
    // A minimal triangulation is one no fill edge can be dropped from, which is
    // not the same as a narrowest one, so the search can land a step above the
    // treewidth. The third case is such a graph. The widths are fixed because
    // the search reads no seed.
    for (rows, columns, expected) in [(3, 3, 3), (4, 4, 4), (3, 6, 4)] {
        let graph = grid(rows, columns);
        let width = minimal_triangulation(&graph).treewidth();
        assert!(
            width >= rows.min(columns),
            "the {rows}x{columns} grid has treewidth {}, and no decomposition is narrower",
            rows.min(columns)
        );
        assert_eq!(width, expected, "the {rows}x{columns} grid");
    }
}

#[test]
fn complete_bipartite_graphs_are_triangulated_near_their_treewidth() {
    for (left, right, expected) in [(2, 5, 2), (3, 3, 3), (3, 4, 4)] {
        let graph = complete_bipartite(left, right);
        let width = minimal_triangulation(&graph).treewidth();
        assert!(
            width >= left.min(right),
            "K_{left},{right} has treewidth {}, and no decomposition is narrower",
            left.min(right)
        );
        assert_eq!(width, expected, "K_{left},{right}");
    }
}

#[test]
fn a_complete_graph_needs_no_fill() {
    let n = 6;
    let edges = (0..n).flat_map(|a| (a + 1..n).map(move |b| (a, b)));
    let graph = Graph::new(n, edges);
    assert_eq!(minimal_triangulation(&graph).treewidth(), n - 1);
}

#[test]
fn the_order_repeats() {
    let graph = grid(5, 6);
    let first = minimal_triangulation(&graph);
    for seed in [0, 1, 12_345] {
        let again = decompose(&graph, Order::MinimalTriangulation, seed, None)
            .expect("a deterministic order takes no weights");
        assert_eq!(again, first, "MCS-M reads no seed, so every run agrees");
    }
}

#[test]
fn maximum_cardinality_search_orders_a_chordal_graph_perfectly() {
    // On a chordal graph the numbering read backwards is a perfect elimination
    // ordering: when a vertex leaves, the neighbours still present are already
    // a clique, so eliminating it adds no edge.
    for (n, k) in [(12, 3), (20, 4), (9, 1)] {
        let graph = k_tree(n, k);
        let adjacency = adjacency_of(&graph);
        let selected = cardinality_search(&adjacency, Reach::Neighbours, Ties::SmallestIndex, None)
            .expect("no deadline stops the search");
        assert_eq!(selected.len(), n as usize);
        let mut position = vec![0usize; n as usize];
        for (index, &vertex) in selected.iter().enumerate() {
            // The elimination order is the numbering reversed, so a higher
            // number leaves earlier.
            position[vertex as usize] = n as usize - index;
        }
        for (vertex, neighbours) in adjacency.iter().enumerate() {
            let later: Vec<u32> = neighbours
                .iter()
                .copied()
                .filter(|&other| position[other as usize] > position[vertex])
                .collect();
            for (index, &a) in later.iter().enumerate() {
                for &b in &later[index + 1..] {
                    assert!(
                        adjacency[a as usize].contains(&b),
                        "the {k}-tree on {n} vertices needs fill at {vertex}: {a} and {b}"
                    );
                }
            }
        }
    }
}

#[test]
fn maximum_cardinality_search_decomposes_a_chordal_graph_at_its_treewidth() {
    for (n, k) in [(12, 3), (20, 4), (9, 1)] {
        assert_eq!(maximum_cardinality(&k_tree(n, k)).treewidth(), k);
    }
}

#[test]
fn maximum_cardinality_search_decomposes_graphs_at_or_above_their_treewidth() {
    // The plain search adds whatever fill its numbering needs and promises no
    // minimality, so all that holds on a graph that is not chordal is that the
    // result is a decomposition and no narrower than the treewidth.
    for n in [4, 7, 25] {
        assert!(
            maximum_cardinality(&cycle(n)).treewidth() >= 2,
            "cycle of {n}"
        );
    }
    for (rows, columns) in [(3, 3), (4, 4), (3, 6)] {
        let width = maximum_cardinality(&grid(rows, columns)).treewidth();
        assert!(width >= rows.min(columns), "the {rows}x{columns} grid");
    }
    for (left, right) in [(2, 5), (3, 3), (3, 4)] {
        let width = maximum_cardinality(&complete_bipartite(left, right)).treewidth();
        assert!(width >= left.min(right), "K_{left},{right}");
    }
}

#[test]
fn the_maximum_cardinality_order_repeats() {
    let graph = grid(5, 6);
    let first = maximum_cardinality(&graph);
    for seed in [0, 1, 12_345] {
        let again = decompose(&graph, Order::MaximumCardinality, seed, None)
            .expect("a deterministic order takes no weights");
        assert_eq!(
            again, first,
            "maximum cardinality search reads no seed, so every run agrees"
        );
    }
}

/// The pick a step makes, worked out the way the search used to: a scan of
/// every unnumbered vertex, taking the highest count and, at a tie, the
/// smallest index.
fn scanned_order(adjacency: &[Vec<u32>]) -> Vec<u32> {
    let n = adjacency.len();
    let mut numbered = vec![false; n];
    let mut count = vec![0u32; n];
    let mut selected = Vec::with_capacity(n);
    for _ in 0..n {
        let mut chosen = usize::MAX;
        for vertex in 0..n {
            if !numbered[vertex] && (chosen == usize::MAX || count[vertex] > count[chosen]) {
                chosen = vertex;
            }
        }
        numbered[chosen] = true;
        selected.push(chosen as u32);
        for &neighbour in &adjacency[chosen] {
            if !numbered[neighbour as usize] {
                count[neighbour as usize] += 1;
            }
        }
    }
    selected
}

/// The heap the plain search picks from has to hand back the vertex a scan of
/// the whole graph would have taken, tie for tie, or the orderings this
/// candidate has been measured on are not the orderings it produces.
#[test]
fn the_plain_search_takes_what_a_scan_would_take() {
    for graph in [
        cycle(9),
        grid(7, 11),
        complete_bipartite(6, 9),
        k_tree(40, 4),
        // Two components, so a step has to choose between vertices whose
        // counts stay at zero for a while.
        Graph::new(
            12,
            [
                (0, 1),
                (1, 2),
                (2, 0),
                (3, 4),
                (4, 5),
                (5, 6),
                (6, 3),
                (7, 8),
            ],
        ),
    ] {
        let adjacency = adjacency_of(&graph);
        let selected = cardinality_search(&adjacency, Reach::Neighbours, Ties::SmallestIndex, None)
            .expect("no deadline to stop the search");
        assert_eq!(
            selected,
            scanned_order(&adjacency),
            "the heap and the scan disagree on a {}-vertex graph",
            graph.num_vertices
        );
    }
}

/// Both reaches under a hard deadline, on a grid neither can number inside the
/// budgets asked for here. The meter is armed, so the budget is a work budget
/// and the run is the same on any machine. The plain search charges the depth
/// of a heap rather than a scan of the graph, so it takes a much larger grid
/// to keep it inside the same budgets.
#[test]
fn a_deadline_stops_either_reach_of_the_search() {
    let paths_graph = grid(160, 160);
    let plain_graph = grid(500, 500);
    let paths_adjacency = adjacency_of(&paths_graph);
    let plain_adjacency = adjacency_of(&plain_graph);
    for (name, reach) in [
        ("maximum cardinality search", Reach::Neighbours),
        ("MCS-M", Reach::LowerPaths),
    ] {
        let adjacency = match reach {
            Reach::Neighbours => &plain_adjacency,
            Reach::LowerPaths => &paths_adjacency,
        };
        for budget_ms in [0, 1, 2, 4, 8] {
            let epoch = Instant::now();
            let guard = crate::meter::arm(epoch);
            let start = crate::meter::units_spent();
            let selected = cardinality_search(
                adjacency,
                reach,
                Ties::SmallestIndex,
                Some(epoch + Duration::from_millis(budget_ms)),
            );
            let spent = crate::meter::units_spent() - start;
            drop(guard);
            assert!(
                selected.is_none(),
                "{name} at {budget_ms} ms: the search numbered the whole grid"
            );
            // Two milliseconds over the budget: the search reads the clock once
            // a millisecond's worth of work has been charged, and ordering the
            // vertices into the heap is charged before the first read.
            let allowed = (budget_ms + 2) * crate::meter::UNITS_PER_MS;
            assert!(
                spent <= allowed,
                "{name} at {budget_ms} ms: the search charged {spent} units, over {allowed}"
            );
        }
    }
    // Neither reach gives up on a graph it has the time for, so it is the
    // budget that stopped them above. A smaller grid, since MCS-M with no
    // deadline costs a walk of the graph per vertex.
    let small = adjacency_of(&grid(12, 12));
    for reach in [Reach::Neighbours, Reach::LowerPaths] {
        let selected = cardinality_search(
            &small,
            reach,
            Ties::SmallestIndex,
            Some(Instant::now() + Duration::from_secs(60)),
        )
        .expect("a minute is enough for a 144-vertex grid");
        assert_eq!(selected.len(), small.len());
    }
}

/// A ranked tie rule numbers every vertex once, one seed gives one order, and
/// two seeds give different ones on a graph whose steps have ties to break.
#[test]
fn a_tie_permutation_gives_one_order_per_seed() {
    let graph = grid(12, 12);
    let adjacency = adjacency_of(&graph);
    let n = adjacency.len();
    let mut orders = Vec::new();
    for seed in [1u64, 2, 3, 1] {
        let (rank, of_rank) = super::super::minimal_triangulation::tie_permutation(n, seed);
        for (place, &vertex) in of_rank.iter().enumerate() {
            assert_eq!(
                rank[vertex as usize] as usize, place,
                "rank inverts of_rank"
            );
        }
        let selected = cardinality_search(
            &adjacency,
            Reach::Neighbours,
            Ties::Ranked {
                rank: &rank,
                of_rank: &of_rank,
            },
            None,
        )
        .expect("no deadline to stop the search");
        let mut seen = selected.clone();
        seen.sort_unstable();
        assert_eq!(
            seen,
            (0..n as u32).collect::<Vec<_>>(),
            "seed {seed} numbered every vertex exactly once"
        );
        orders.push(selected);
    }
    assert_eq!(orders[0], orders[3], "one seed gives one order");
    assert_ne!(orders[0], orders[1], "two seeds give different orders");
    assert_ne!(orders[1], orders[2], "two seeds give different orders");
    let plain = cardinality_search(&adjacency, Reach::Neighbours, Ties::SmallestIndex, None)
        .expect("no deadline to stop the search");
    assert_ne!(
        plain, orders[0],
        "a ranked tie rule is not the index tie rule"
    );
}
