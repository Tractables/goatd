use crate::elimination::execution::{ElimSink, ElimStop};
use crate::elimination::graph::EliminationGraph;
use crate::elimination::greedy::min_fill::*;
use crate::elimination::greedy::sampling::eliminate_sampled_min_fill;

#[test]
fn path_graph_eliminates_from_endpoints() {
    let mut g = EliminationGraph::from_edges(4, &[(0, 1), (1, 2), (2, 3)]);
    let salt = vec![0u32; 4];
    let mut bags = Vec::new();
    let mut rank = Vec::new();
    let sink = ElimSink::new(&mut bags, &mut rank, 0);
    eliminate_min_fill(&mut g, &salt, sink, ElimStop::default());
    assert_eq!(bags.len(), 4);
    let first = bags[0][0];
    assert!(first == 0 || first == 3);
    assert_eq!(g.num_active, 0);
}

#[test]
fn triangle_eliminates_in_three_steps() {
    let mut g = EliminationGraph::from_edges(3, &[(0, 1), (0, 2), (1, 2)]);
    let salt = vec![0u32; 3];
    let mut bags = Vec::new();
    let mut rank = Vec::new();
    let sink = ElimSink::new(&mut bags, &mut rank, 0);
    eliminate_min_fill(&mut g, &salt, sink, ElimStop::default());
    assert_eq!(bags.len(), 3);
    assert_eq!(bags[0].len(), 3);
}

#[test]
fn min_fill_rechecks_vertices_two_hops_from_an_elimination() {
    let edges = [
        (0, 3),
        (0, 4),
        (0, 5),
        (1, 3),
        (1, 4),
        (1, 5),
        (2, 3),
        (2, 4),
        (2, 5),
    ];
    let mut graph = EliminationGraph::from_edges(6, &edges);
    let salt = vec![0; 6];
    let mut bags = Vec::new();
    let mut rank = Vec::new();
    let sink = ElimSink::new(&mut bags, &mut rank, 0);

    eliminate_min_fill(&mut graph, &salt, sink, ElimStop::default());

    assert_eq!(bags[0][0], 0);
    assert_eq!(
        bags[1][0], 1,
        "eliminating 0 makes vertex 1 simplicial, so it must precede a vertex with positive fill",
    );
}

#[test]
fn sampled_min_fill_rechecks_vertices_two_hops_from_an_elimination() {
    let edges = [
        (0, 3),
        (0, 4),
        (0, 5),
        (1, 3),
        (1, 4),
        (1, 5),
        (2, 3),
        (2, 4),
        (2, 5),
    ];
    let mut graph = EliminationGraph::from_edges(6, &edges);
    let weights = vec![1; 6];
    let mut bags = Vec::new();
    let mut rank = Vec::new();
    let sink = ElimSink::new(&mut bags, &mut rank, 0);

    eliminate_sampled_min_fill(&mut graph, &weights, 0, sink, ElimStop::default(), None);

    let mut reference = EliminationGraph::from_edges(6, &edges);
    for (step, bag) in bags.iter().enumerate() {
        let selected = bag[0];
        let selected_fill = reference.fill_count_of_bs(selected);
        let minimum_fill = (0..6)
            .filter(|&vertex| reference.active[vertex])
            .map(|vertex| reference.fill_count_of_bs(vertex as u32))
            .min()
            .unwrap();
        assert_eq!(
            selected_fill, minimum_fill,
            "step {step} selected vertex {selected} with fill {selected_fill}, minimum {minimum_fill}",
        );
        reference.eliminate(selected);
    }
}
