use std::time::Duration;

use goatd::elimination::{Order, decompose};
use goatd::portfolio::{PortfolioConfig, candidates};
use goatd::{Graph, TreeDecomposition};

#[test]
fn graph_and_decomposition_are_readable_without_exposing_storage() {
    let graph = Graph::new(4, [(1, 0), (2, 1), (0, 1), (2, 2)]);
    assert_eq!(graph.num_vertices(), 4);
    assert_eq!(graph.edges(), [(0, 1), (1, 2)]);

    let td = TreeDecomposition::new(&graph, [vec![1, 0], vec![2, 1], vec![3]], [(0, 1)])
        .expect("a decomposition forest of the graph");
    assert_eq!(td.num_vertices(), 4);
    assert_eq!(td.bags()[0].vertices(), [0, 1]);
    assert_eq!(td.adjacency()[0], [1]);
    assert!(td.to_td().starts_with("s td 3 2 4\n"));
}

#[test]
fn decomposition_constructor_checks_the_graph_contract() {
    let graph = Graph::new(3, [(0, 2)]);
    let error = TreeDecomposition::new(&graph, [vec![0, 1], vec![1, 2]], [(0, 1)])
        .expect_err("no bag covers edge (0, 2)")
        .to_string();
    assert!(error.contains("edge (0, 2)") && error.contains("no bag"));
}

#[test]
#[should_panic(expected = "endpoint outside 0..3")]
fn graph_constructor_rejects_out_of_range_endpoints() {
    let _ = Graph::new(3, [(0, 3)]);
}

#[test]
fn fallible_graph_construction_reports_out_of_range_endpoints() {
    let error = Graph::try_new(3, [(0, 3)]).unwrap_err();

    assert!(error.to_string().contains("edge (0, 3)"));
    assert!(error.to_string().contains("outside 0..3"));
}

#[test]
fn induced_subgraph_is_a_self_contained_public_operation() {
    let graph = Graph::new(5, [(0, 3), (3, 4), (1, 4)]);
    let induced = graph.induced_subgraph(&[4, 3, 0]).unwrap();

    assert_eq!(induced.edges(), [(0, 1), (1, 2)]);
}

#[test]
fn decomposition_projection_owns_its_mapping() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3)]);
    let td = TreeDecomposition::new(
        &graph,
        [vec![0, 1], vec![1, 2], vec![2, 3]],
        [(0, 1), (1, 2)],
    )
    .unwrap();

    let projection = td.project(&[3, 2]).unwrap();
    assert_eq!(projection.local_to_original(), [2, 3]);
    assert_eq!(projection.decomposition().bags()[0].vertices(), [0]);
    assert_eq!(projection.decomposition().bags()[1].vertices(), [0, 1]);

    let (decomposition, local_to_original) = projection.into_parts();
    assert_eq!(decomposition.num_vertices(), 2);
    assert_eq!(local_to_original, [2, 3]);
}

#[test]
fn decomposition_rooting_rejects_an_unknown_bag() {
    let graph = Graph::new(1, []);
    let td = TreeDecomposition::new(&graph, [vec![0]], []).unwrap();

    assert!(td.rooted_forest([1]).is_err());
}

#[test]
fn decomposition_rooting_visits_every_component() {
    let graph = Graph::new(3, []);
    let td = TreeDecomposition::new(&graph, [vec![0], vec![1], vec![2]], []).unwrap();
    let rooted = td.rooted_forest([2]).unwrap();

    assert_eq!(rooted.order(), [2, 0, 1]);
    assert_eq!(rooted.parents(), [None, None, None]);
    assert_eq!(rooted.depths(), [0, 0, 0]);
    assert_eq!(rooted.component_roots(), [2, 0, 1]);
}

#[test]
fn decomposition_refinement_rejects_a_decomposition_of_another_graph() {
    let one = Graph::new(1, []);
    let two = Graph::new(2, []);
    let td = TreeDecomposition::new(&one, [vec![0]], []).unwrap();

    assert!(goatd::decomposition::refine_with_flowcutter(td, &two, None).is_err());
}

#[test]
fn sampled_constructions_check_the_weight_count() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);
    let weights = [1, 1];

    assert!(decompose(&graph, Order::MinFillSampled { weights: &weights }, 0, None,).is_err());
    assert!(candidates(&graph, &weights, 0, PortfolioConfig::standard()).is_err());
}

#[test]
fn elimination_rejects_an_unrepresentable_budget() {
    let graph = Graph::new(1, []);

    assert!(decompose(&graph, Order::MinFill, 0, Some(Duration::MAX)).is_err());
}
