use std::time::{Duration, Instant};

use super::sets::Adjacency;
use super::{BagPool, Limits, recombine};
use crate::{Graph, TreeDecomposition};

fn pool_of(decompositions: &[&TreeDecomposition]) -> BagPool {
    let mut pool = BagPool::new(Limits::standard());
    for decomposition in decompositions {
        pool.absorb(decomposition, 0);
    }
    pool
}

/// A path of `n` vertices.
fn path(n: u32) -> Graph {
    Graph::new(n, (0..n.saturating_sub(1)).map(|v| (v, v + 1)))
}

/// The 3 × 3 grid, vertex `3 * row + column`. Its treewidth is 3.
fn grid() -> Graph {
    let mut edges = Vec::new();
    for row in 0..3u32 {
        for column in 0..3u32 {
            let vertex = row * 3 + column;
            if column + 1 < 3 {
                edges.push((vertex, vertex + 1));
            }
            if row + 1 < 3 {
                edges.push((vertex, vertex + 3));
            }
        }
    }
    Graph::new(9, edges)
}

#[test]
fn an_empty_pool_gives_nothing() {
    let graph = path(4);
    let pool = BagPool::new(Limits::standard());
    assert!(recombine(&pool, &graph, None).is_none());
}

#[test]
fn the_pool_keeps_the_best_few_decompositions() {
    let graph = path(4);
    let narrow = TreeDecomposition::new(
        &graph,
        [vec![0, 1], vec![1, 2], vec![2, 3]],
        [(0, 1), (1, 2)],
    )
    .unwrap();
    let wide = TreeDecomposition::new(&graph, [vec![0, 1, 2, 3]], []).unwrap();
    let mut pool = BagPool::new(Limits::standard());
    for slot in 0..8 {
        pool.absorb(&wide, slot);
        pool.absorb(&narrow, slot);
    }
    // One decomposition per slot, and each slot kept the narrower of the two
    // offered to it.
    assert_eq!(pool.len(), 8);
    let built = recombine(&pool, &graph, None).expect("the pool holds a decomposition");
    assert_eq!(built.treewidth(), 1);
}

/// Every stage that produced a decomposition is in the pool, and when they do
/// not all fit, each still gives up bags rather than being dropped.
#[test]
fn a_full_pool_shares_its_bags_out() {
    let graph = path(9);
    let path_bags: Vec<Vec<u32>> = (0..8).map(|v| vec![v, v + 1]).collect();
    let along = TreeDecomposition::new(
        &graph,
        path_bags.clone(),
        (0usize..7).map(|edge| (edge, edge + 1)),
    )
    .unwrap();
    let mut pool = BagPool::new(Limits {
        bags: 4,
        ..Limits::standard()
    });
    for slot in 0..4 {
        pool.absorb(&along, slot);
    }
    assert_eq!(pool.len(), 4);
    let built = recombine(&pool, &graph, None).expect("the pool holds a decomposition");
    built.validate(&graph).expect("valid");
}

#[test]
fn a_pool_from_one_decomposition_rebuilds_it() {
    let graph = path(6);
    let td = TreeDecomposition::new(
        &graph,
        [vec![0, 1], vec![1, 2], vec![2, 3], vec![3, 4], vec![4, 5]],
        [(0, 1), (1, 2), (2, 3), (3, 4)],
    )
    .unwrap();
    let pool = pool_of(&[&td]);
    let built = recombine(&pool, &graph, None).expect("the pool holds a whole decomposition");
    built.validate(&graph).expect("valid");
    assert_eq!(built.treewidth(), 1);
}

#[test]
fn the_search_never_comes_back_wider_than_the_pool_it_read() {
    // A four-cycle with a chord: treewidth 2, and a decomposition of width 3
    // in the pool as well.
    let graph = Graph::new(5, [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2), (2, 4)]);
    let wide = TreeDecomposition::new(&graph, [vec![0, 1, 2, 3], vec![2, 4]], [(0, 1)]).unwrap();
    let narrow = TreeDecomposition::new(
        &graph,
        [vec![0, 1, 2], vec![0, 2, 3], vec![2, 4]],
        [(0, 1), (0, 2)],
    )
    .unwrap();
    let pool = pool_of(&[&wide, &narrow]);
    let built = recombine(&pool, &graph, None).expect("the pool holds a whole decomposition");
    built.validate(&graph).expect("valid");
    assert!(built.treewidth() <= narrow.treewidth());
}

/// Two decompositions of the same graph, each narrow on one half and wide on
/// the other. Neither is narrower than the other overall, and the bags that
/// make a narrow decomposition of the whole graph are split between them.
#[test]
fn it_glues_a_narrower_tree_out_of_two_candidates() {
    // Two four-cycles with a chord each, joined at a cut vertex. Each cycle is
    // decomposed by its two triangles, or badly by one bag of four.
    //   left:  0-1-2-3-0 with the chord 0-2
    //   right: 3-4-5-6-3 with the chord 3-5
    let graph = Graph::new(
        7,
        [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (0, 2),
            (3, 4),
            (4, 5),
            (5, 6),
            (6, 3),
            (3, 5),
        ],
    );
    // Narrow on the left, wide on the right.
    let left = TreeDecomposition::new(
        &graph,
        [vec![0, 1, 2], vec![0, 2, 3], vec![3, 4, 5, 6]],
        [(0, 1), (1, 2)],
    )
    .unwrap();
    // Wide on the left, narrow on the right.
    let right = TreeDecomposition::new(
        &graph,
        [vec![0, 1, 2, 3], vec![3, 4, 5], vec![3, 5, 6]],
        [(0, 1), (1, 2)],
    )
    .unwrap();
    assert_eq!(left.treewidth(), 3);
    assert_eq!(right.treewidth(), 3);
    // The bags of the two, read by the programme as they are: the pool would
    // minimalise them first, and this is about what the search does with a
    // list, not about what minimalisation does to one decomposition.
    let mut bags: Vec<Vec<u32>> = Vec::new();
    for decomposition in [&left, &right] {
        for bag in decomposition.bags() {
            let mut vertices = bag.vertices().to_vec();
            vertices.sort_unstable();
            if !bags.contains(&vertices) {
                bags.push(vertices);
            }
        }
    }
    let adjacency = Adjacency::of(&graph).expect("the rows fit");
    let sets: Vec<_> = bags.iter().map(|bag| adjacency.set_of(bag)).collect();
    let built = super::search(&sets, &graph, &adjacency, Limits::standard(), None)
        .expect("the bags hold a whole decomposition");
    built.validate(&graph).expect("valid");
    // Neither candidate is narrower than 3; the four triangles between them
    // are a decomposition of width 2, and no candidate had them together.
    assert_eq!(built.treewidth(), 2);
}

#[test]
fn a_passed_deadline_gives_nothing() {
    let graph = path(6);
    let td = TreeDecomposition::new(
        &graph,
        [vec![0, 1], vec![1, 2], vec![2, 3], vec![3, 4], vec![4, 5]],
        [(0, 1), (1, 2), (2, 3), (3, 4)],
    )
    .unwrap();
    let pool = pool_of(&[&td]);
    let past = Instant::now() - Duration::from_secs(1);
    assert!(recombine(&pool, &graph, Some(past)).is_none());
}

#[test]
fn a_disconnected_graph_comes_back_whole() {
    let graph = Graph::new(5, [(0, 1), (1, 2), (3, 4)]);
    let td = TreeDecomposition::new(
        &graph,
        [vec![0, 1], vec![1, 2], vec![3, 4]],
        [(0, 1), (1, 2)],
    )
    .unwrap();
    let pool = pool_of(&[&td]);
    let built = recombine(&pool, &graph, None).expect("the pool holds a whole decomposition");
    built.validate(&graph).expect("valid");
    assert_eq!(built.treewidth(), 1);
}

/// The search stops taking bags in when it reaches its cap. What it has by
/// then is still a decomposition of the whole graph, or nothing.
#[test]
fn a_full_block_map_gives_a_valid_answer_or_none() {
    let graph = path(8);
    let td = TreeDecomposition::new(
        &graph,
        (0u32..7).map(|v| vec![v, v + 1]).collect::<Vec<_>>(),
        (0usize..6).map(|edge| (edge, edge + 1)),
    )
    .unwrap();
    for block_vertices in 0..64 {
        let mut pool = BagPool::new(Limits {
            block_vertices,
            ..Limits::standard()
        });
        pool.absorb(&td, 0);
        if let Some(built) = recombine(&pool, &graph, None) {
            built.validate(&graph).expect("valid");
        }
    }
}

/// The growth pass re-decomposes the pieces around the widest bags of an
/// answer and adds what it finds, so the second run has bags the pool never
/// held.
#[test]
fn growth_adds_bags_the_pool_did_not_hold() {
    // Two chorded four-cycles joined at a vertex again, but the pool is given
    // only the coarse decomposition: one bag per cycle.
    let graph = Graph::new(
        7,
        [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (0, 2),
            (3, 4),
            (4, 5),
            (5, 6),
            (6, 3),
            (3, 5),
        ],
    );
    let coarse =
        TreeDecomposition::new(&graph, [vec![0, 1, 2, 3], vec![3, 4, 5, 6]], [(0, 1)]).unwrap();
    let mut bags: Vec<Vec<u32>> = coarse
        .bags()
        .iter()
        .map(|bag| bag.vertices().to_vec())
        .collect();
    let held = bags.len();
    let adjacency = Adjacency::of(&graph).expect("the rows fit");
    let mut sets: Vec<_> = bags.iter().map(|bag| adjacency.set_of(bag)).collect();
    assert!(super::grow(
        &mut sets,
        &coarse,
        &adjacency,
        Limits::standard(),
        None
    ));
    assert!(sets.len() > held);
    let built = super::search(&sets, &graph, &adjacency, Limits::standard(), None)
        .expect("the longer list holds a decomposition");
    built.validate(&graph).expect("valid");
    assert!(built.treewidth() <= coarse.treewidth());
}

/// The Bouchitté–Todinca test, on sets whose status can be read off the graph.
#[test]
fn the_potential_maximal_clique_test_agrees_with_small_graphs() {
    // A four-cycle: its minimal separators are {0,2} and {1,3}, and its
    // potential maximal cliques are the four triangles, since every minimal
    // triangulation adds one chord.
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3), (3, 0)]);
    let adjacency = Adjacency::of(&graph).expect("the rows fit");
    let mut scratch = super::sets::Scratch::new(&adjacency);
    for triple in [vec![0, 1, 2], vec![1, 2, 3], vec![0, 2, 3], vec![0, 1, 3]] {
        assert!(super::merge::is_potential_maximal_clique(
            &triple,
            &adjacency,
            &mut scratch
        ));
    }
    // A single edge is a minimal separator, so the two components either side
    // of {0,2} both have it whole on their border and it is not one.
    assert!(!super::merge::is_potential_maximal_clique(
        &[0, 2],
        &adjacency,
        &mut scratch
    ));
    // The whole graph is: nothing is left outside it, and both non-adjacent
    // pairs would have to be covered by a component that does not exist — but
    // there is no component at all, so the pairs are uncovered.
    assert!(!super::merge::is_potential_maximal_clique(
        &[0, 1, 2, 3],
        &adjacency,
        &mut scratch
    ));
}

#[test]
fn the_merge_loop_never_comes_back_wider_than_it_started() {
    let graph = grid();
    let wide = TreeDecomposition::new(
        &graph,
        [vec![0, 1, 2, 3, 4, 5], vec![3, 4, 5, 6, 7, 8]],
        [(0, 1)],
    )
    .unwrap();
    let found = super::merge_loop(
        &graph,
        Some(&wide),
        0,
        Limits::standard(),
        Some(Instant::now() + Duration::from_millis(500)),
    )
    .expect("the loop settles on a small graph");
    found.validate(&graph).expect("a valid decomposition");
    assert!(found.treewidth() <= wide.treewidth());
}

#[test]
fn the_merge_loop_stops_at_a_passed_deadline() {
    let graph = grid();
    let start = TreeDecomposition::new(&graph, [(0..9).collect::<Vec<u32>>()], []).unwrap();
    assert!(
        super::merge_loop(
            &graph,
            Some(&start),
            0,
            Limits::standard(),
            Some(Instant::now() - Duration::from_millis(1)),
        )
        .is_none()
    );
}

#[test]
fn the_merge_loop_decomposes_a_graph_on_its_own() {
    let graph = grid();
    let found = super::decompose_by_merging(&graph, 0, Some(Duration::from_millis(500)))
        .expect("the construction answers");
    found.validate(&graph).expect("a valid decomposition");
    // The 3 x 3 grid has treewidth 3, and a minimal triangulation of it does
    // not do worse.
    assert_eq!(found.treewidth(), 3);
}

#[test]
fn a_disconnected_graph_merges() {
    let graph = Graph::new(6, [(0, 1), (1, 2), (3, 4), (4, 5)]);
    let found = super::decompose_by_merging(&graph, 0, Some(Duration::from_millis(500)))
        .expect("the construction answers");
    found.validate(&graph).expect("a valid decomposition");
    assert_eq!(found.treewidth(), 1);
}

/// Three graphs and, for each, a fixed list of bags: what the programme reads
/// off a list is a property of the list and the graph, so these numbers hold
/// whatever the sets are held in.
fn fixed_lists() -> Vec<(Graph, Vec<Vec<u32>>)> {
    let mut cases = Vec::new();
    for graph in [square_grid(4), square_grid(5), scattered(30)] {
        let mut bags: Vec<Vec<u32>> = Vec::new();
        for order in [
            crate::elimination::Order::MinFill,
            crate::elimination::Order::MinDegree,
            crate::elimination::Order::MinimalTriangulation,
        ] {
            let decomposition =
                crate::elimination::decompose(&graph, order, 0, None).expect("an order");
            for bag in decomposition.bags() {
                let mut vertices = bag.vertices().to_vec();
                vertices.sort_unstable();
                if !bags.contains(&vertices) {
                    bags.push(vertices);
                }
            }
        }
        cases.push((graph, bags));
    }
    cases
}

/// The `n × n` grid, vertex `n * row + column`.
fn square_grid(n: u32) -> Graph {
    let mut edges = Vec::new();
    for row in 0..n {
        for column in 0..n {
            let vertex = row * n + column;
            if column + 1 < n {
                edges.push((vertex, vertex + 1));
            }
            if row + 1 < n {
                edges.push((vertex, vertex + n));
            }
        }
    }
    Graph::new(n * n, edges)
}

/// A graph with no structure to it, built from a fixed rule so the list the
/// test reads is the same on every machine.
fn scattered(n: u32) -> Graph {
    let mut edges = Vec::new();
    for left in 0..n {
        for right in left + 1..n {
            if (left * 7 + right * 13 + left * right) % 11 < 3 {
                edges.push((left, right));
            }
        }
    }
    Graph::new(n, edges)
}

#[test]
fn the_programme_reads_the_widths_it_always_read() {
    let expected = [(4u32, 48usize), (5, 87), (13, 150)];
    for ((graph, bags), (width, total)) in fixed_lists().into_iter().zip(expected) {
        let adjacency = Adjacency::of(&graph).expect("the rows fit");
        let sets: Vec<_> = bags.iter().map(|bag| adjacency.set_of(bag)).collect();
        let built = super::search(&sets, &graph, &adjacency, Limits::standard(), None)
            .expect("the list holds a decomposition");
        built.validate(&graph).expect("valid");
        assert_eq!(built.quality_key(), (width, total));
    }
}
