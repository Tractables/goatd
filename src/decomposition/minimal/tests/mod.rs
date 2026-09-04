//! The minimalization pass against a brute-force definition of minimality.

use crate::decomposition::minimalize_triangulation;
use crate::elimination::minimal_triangulation::{Reach, cardinality_search};
use crate::elimination::{Order, decompose};
use crate::rng::{SEED_OFFSET, Xorshift64};
use crate::{Graph, TreeDecomposition};

/// One adjacency list per vertex, built by completing every bag to a clique.
fn completion(decomposition: &TreeDecomposition) -> Vec<Vec<u32>> {
    let vertices = decomposition.num_vertices() as usize;
    let mut adjacency = vec![Vec::new(); vertices];
    for bag in decomposition.bags() {
        let bag = bag.vertices();
        for (position, &left) in bag.iter().enumerate() {
            for &right in &bag[position + 1..] {
                if !adjacency[left as usize].contains(&right) {
                    adjacency[left as usize].push(right);
                    adjacency[right as usize].push(left);
                }
            }
        }
    }
    for row in &mut adjacency {
        row.sort_unstable();
    }
    adjacency
}

fn adjacency_of(graph: &Graph) -> Vec<Vec<u32>> {
    let mut adjacency = vec![Vec::new(); graph.num_vertices() as usize];
    for &(left, right) in graph.edges() {
        if left != right && !adjacency[left as usize].contains(&right) {
            adjacency[left as usize].push(right);
            adjacency[right as usize].push(left);
        }
    }
    for row in &mut adjacency {
        row.sort_unstable();
    }
    adjacency
}

/// Whether a graph given as adjacency lists is chordal, by checking that the
/// ordering a maximum cardinality search produces is a perfect elimination
/// ordering.
fn is_chordal(adjacency: &[Vec<u32>]) -> bool {
    let selected = cardinality_search(adjacency, Reach::Neighbours, None)
        .expect("an unbounded search always finishes");
    let mut rank = vec![0usize; adjacency.len()];
    for (step, &vertex) in selected.iter().rev().enumerate() {
        rank[vertex as usize] = step;
    }
    for vertex in 0..adjacency.len() {
        let later: Vec<u32> = adjacency[vertex]
            .iter()
            .copied()
            .filter(|&other| rank[other as usize] > rank[vertex])
            .collect();
        for (position, &left) in later.iter().enumerate() {
            for &right in &later[position + 1..] {
                if !adjacency[left as usize].contains(&right) {
                    return false;
                }
            }
        }
    }
    true
}

fn remove_edge(adjacency: &[Vec<u32>], left: u32, right: u32) -> Vec<Vec<u32>> {
    let mut reduced = adjacency.to_vec();
    reduced[left as usize].retain(|&other| other != right);
    reduced[right as usize].retain(|&other| other != left);
    reduced
}

/// A random graph on `vertices` vertices where each pair is an edge with
/// probability `numerator / 16`.
fn random_graph(vertices: u32, numerator: u32, seed: u64) -> Graph {
    let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
    let mut edges = Vec::new();
    for left in 0..vertices {
        for right in left + 1..vertices {
            if rng.next_u32() % 16 < numerator {
                edges.push((left, right));
            }
        }
    }
    Graph::new(vertices, edges)
}

#[test]
fn the_pass_never_widens_and_leaves_a_minimal_triangulation() {
    for seed in 0..40u64 {
        for numerator in [3, 6, 10] {
            let graph = random_graph(11, numerator, seed);
            let before = decompose(&graph, Order::MinDegree, seed, None)
                .expect("a deterministic order takes no weights");
            let after = minimalize_triangulation(before.clone(), &graph, None)
                .expect("the pass is given its own graph");
            after
                .validate(&graph)
                .expect("the pass returns a decomposition of the same graph");
            assert!(
                after.treewidth() <= before.treewidth(),
                "seed {seed}: the pass widened {} to {}",
                before.treewidth(),
                after.treewidth()
            );

            let filled = completion(&after);
            let original = adjacency_of(&graph);
            assert!(
                is_chordal(&filled),
                "seed {seed}: the result is not chordal"
            );
            for left in 0..graph.num_vertices() {
                for &right in &filled[left as usize] {
                    if right <= left || original[left as usize].contains(&right) {
                        continue;
                    }
                    assert!(
                        !is_chordal(&remove_edge(&filled, left, right)),
                        "seed {seed}: fill edge ({left}, {right}) could still be dropped"
                    );
                }
            }
        }
    }
}

#[test]
fn a_chordal_graph_keeps_its_decomposition() {
    // A path is chordal, so its min-degree decomposition adds no fill and the
    // pass has nothing to drop.
    let graph = Graph::new(8, (0..7).map(|vertex| (vertex, vertex + 1)));
    let before = decompose(&graph, Order::MinDegree, 0, None)
        .expect("a deterministic order takes no weights");
    let after = minimalize_triangulation(before.clone(), &graph, None)
        .expect("the pass is given its own graph");
    assert_eq!(after, before);
}

#[test]
fn the_pass_repeats() {
    let graph = random_graph(14, 5, 7);
    let before = decompose(&graph, Order::MinDegree, 7, None)
        .expect("a deterministic order takes no weights");
    let first = minimalize_triangulation(before.clone(), &graph, None)
        .expect("the pass is given its own graph");
    let second =
        minimalize_triangulation(before, &graph, None).expect("the pass is given its own graph");
    assert_eq!(first, second);
}

#[test]
fn a_decomposition_of_another_graph_is_refused() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3)]);
    let other = Graph::new(4, [(0, 2), (1, 3)]);
    let td = decompose(&graph, Order::MinDegree, 0, None)
        .expect("a deterministic order takes no weights");
    assert!(minimalize_triangulation(td, &other, None).is_err());
}
