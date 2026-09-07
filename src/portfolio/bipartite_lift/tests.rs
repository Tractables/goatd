use std::time::Duration;

use super::*;
use crate::portfolio::{PortfolioConfig, Stage};

/// The incidence graph of a small formula: variables `0..v`, one vertex per
/// clause after them, an edge from each clause to its variables.
fn incidence(variables: u32, clauses: &[&[u32]]) -> Graph {
    let edges = clauses.iter().enumerate().flat_map(|(index, clause)| {
        let clause_vertex = variables + index as u32;
        clause.iter().map(move |&variable| (variable, clause_vertex))
    });
    Graph::new(variables + clauses.len() as u32, edges)
}

fn sides_of(graph: &Graph) -> Option<[Vec<u32>; 2]> {
    sides(&adjacency(graph))
}

#[test]
fn colours_a_bipartite_graph_and_refuses_an_odd_cycle() {
    let path = Graph::new(4, [(0, 1), (1, 2), (2, 3)]);
    let [left, right] = sides_of(&path).expect("a path is bipartite");
    assert_eq!((left, right), (vec![0, 2], vec![1, 3]));

    let triangle = Graph::new(3, [(0, 1), (1, 2), (0, 2)]);
    assert!(sides_of(&triangle).is_none());
}

#[test]
fn projecting_the_clause_side_gives_the_primal_graph() {
    // Two clauses over three variables: {0,1,2} and {1,2}.
    let graph = incidence(3, &[&[0, 1, 2], &[1, 2]]);
    let adjacency = adjacency(&graph);
    let projection = project(&graph, &adjacency, &[0, 1, 2], &[3, 4], usize::MAX)
        .expect("the projection fits");
    assert_eq!(projection.graph.num_vertices(), 3);
    assert_eq!(projection.graph.edges().to_vec(), vec![(0u32, 1), (0, 2), (1, 2)]);
    // The widest clause holds three variables, so the side contributes 3.
    assert_eq!(projection.eliminated_width, 3);
}

#[test]
fn refuses_a_projection_over_the_edge_limit() {
    // One clause over six variables projects to fifteen edges.
    let graph = incidence(6, &[&[0, 1, 2, 3, 4, 5]]);
    let adjacency = adjacency(&graph);
    assert!(project(&graph, &adjacency, &[0, 1, 2, 3, 4, 5], &[6], 14).is_none());
    assert!(project(&graph, &adjacency, &[0, 1, 2, 3, 4, 5], &[6], 15).is_some());
}

#[test]
fn lifts_a_decomposition_of_the_projection() {
    // A chain of clauses, so the primal graph is a chain of triangles and its
    // decomposition is more than one bag.
    let graph = incidence(5, &[&[0, 1, 2], &[1, 2, 3], &[2, 3, 4], &[0, 4]]);
    let adjacency = adjacency(&graph);
    let keep: Vec<u32> = (0..5).collect();
    let drop: Vec<u32> = (5..9).collect();
    let projection =
        project(&graph, &adjacency, &keep, &drop, usize::MAX).expect("the projection fits");
    let projected = crate::elimination::decompose(
        &projection.graph,
        crate::elimination::Order::MinFill,
        0,
        None,
    )
    .expect("the projection decomposes");

    let lifted = projection
        .lift(&graph, &projected)
        .expect("every neighbourhood is a clique of the projection");
    lifted
        .validate(&graph)
        .expect("the lift is a decomposition of the whole graph");
    // The construction's width, and the promise the stage is built on.
    assert_eq!(
        lifted.treewidth(),
        projection.lifted_width(&projected),
        "the lift is as wide as the projection or as the widest clause"
    );
    assert!(lifted.treewidth() <= projected.treewidth() + 1);
}

#[test]
fn the_portfolio_lifts_an_incidence_graph() {
    // Ten clauses over six variables: the incidence graph is bipartite and its
    // clause side projects onto the primal graph.
    let clauses: Vec<Vec<u32>> = vec![
        vec![0, 1, 2],
        vec![1, 2, 3],
        vec![2, 3, 4],
        vec![3, 4, 5],
        vec![0, 4, 5],
        vec![0, 2, 5],
        vec![1, 3, 5],
        vec![0, 1, 4],
        vec![2, 4, 5],
        vec![0, 3, 4],
    ];
    let borrowed: Vec<&[u32]> = clauses.iter().map(Vec::as_slice).collect();
    let graph = incidence(6, &borrowed);
    let weights = vec![1; graph.num_vertices() as usize];
    let config = PortfolioConfig::standard_with_budget(Duration::from_millis(200));

    let mut stages = Vec::new();
    let decomposition = crate::portfolio::decompose_traced(&graph, &weights, 0, config, &mut |t| {
        stages.push(t.stage);
    })
    .expect("the portfolio returns a decomposition");
    decomposition
        .validate(&graph)
        .expect("the portfolio's winner is valid");
    assert!(
        stages.contains(&Stage::BipartiteLift),
        "the lift ran on a bipartite graph: {stages:?}"
    );
}

#[test]
fn a_graph_that_is_not_bipartite_is_left_alone() {
    // A triangle with a pendant: no 2-colouring, so the stage reports nothing
    // and the portfolio returns what its orders found.
    let graph = Graph::new(4, [(0, 1), (1, 2), (0, 2), (2, 3)]);
    let weights = vec![1; graph.num_vertices() as usize];
    let config = PortfolioConfig::standard_with_budget(Duration::from_millis(100));

    let mut stages = Vec::new();
    let decomposition = crate::portfolio::decompose_traced(&graph, &weights, 0, config, &mut |t| {
        stages.push(t.stage);
    })
    .expect("the portfolio returns a decomposition");

    assert!(
        !stages.contains(&Stage::BipartiteLift),
        "no lift on a graph with an odd cycle: {stages:?}"
    );
    decomposition
        .validate(&graph)
        .expect("the winner is a decomposition of the graph");
    assert_eq!(decomposition.treewidth(), 2, "the triangle needs one bag");
}
