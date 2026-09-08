use std::time::{Duration, Instant};

use super::{BagPool, Limits, recombine};
use crate::{Graph, TreeDecomposition};

fn pool_of(decompositions: &[&TreeDecomposition]) -> BagPool {
    let mut pool = BagPool::new(Limits::standard());
    for decomposition in decompositions {
        pool.absorb(decomposition);
    }
    pool
}

/// A path of `n` vertices.
fn path(n: u32) -> Graph {
    Graph::new(n, (0..n.saturating_sub(1)).map(|v| (v, v + 1)))
}

#[test]
fn an_empty_pool_gives_nothing() {
    let graph = path(4);
    let pool = BagPool::new(Limits::standard());
    assert!(recombine(&pool, &graph, None).is_none());
}

#[test]
fn the_pool_holds_each_bag_once() {
    let graph = path(4);
    let td = TreeDecomposition::new(
        &graph,
        [vec![0, 1], vec![1, 2], vec![2, 3]],
        [(0, 1), (1, 2)],
    )
    .unwrap();
    let mut pool = pool_of(&[&td]);
    assert_eq!(pool.len(), 3);
    // The same bags again, listed in another order inside each bag.
    let same = TreeDecomposition::new(
        &graph,
        [vec![1, 0], vec![2, 1], vec![3, 2]],
        [(0, 1), (1, 2)],
    )
    .unwrap();
    pool.absorb(&same);
    assert_eq!(pool.len(), 3);
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
    let pool = pool_of(&[&left, &right]);
    let built = recombine(&pool, &graph, None).expect("the pool holds a whole decomposition");
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
        pool.absorb(&td);
        if let Some(built) = recombine(&pool, &graph, None) {
            built.validate(&graph).expect("valid");
        }
    }
}
