use crate::elimination::execution::{Cutoff, ElimExit, ElimSink, ElimStop};
use crate::elimination::graph::EliminationGraph;
use crate::elimination::greedy::min_fill::*;
use crate::elimination::greedy::sampling::{
    eliminate_sampled_fill_degree, eliminate_sampled_min_fill,
};

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

fn assert_sampled_fill_degree_minimizes_score(degree_coefficient: i8) {
    let edges = [
        (0, 1),
        (0, 2),
        (0, 3),
        (1, 2),
        (1, 3),
        (2, 3),
        (0, 4),
        (1, 5),
        (2, 6),
    ];
    let mut graph = EliminationGraph::from_edges(7, &edges);
    let weights = vec![1; 7];
    let mut bags = Vec::new();
    let mut rank = Vec::new();
    let sink = ElimSink::new(&mut bags, &mut rank, 0);

    eliminate_sampled_fill_degree(
        &mut graph,
        &weights,
        0,
        sink,
        ElimStop::default(),
        None,
        degree_coefficient,
    );

    let mut reference = EliminationGraph::from_edges(7, &edges);
    for (step, bag) in bags.iter().enumerate() {
        let selected = bag[0];
        let selected_score = reference.fill_count_of_bs(selected) as i64
            + i64::from(degree_coefficient) * reference.degree(selected) as i64;
        let minimum_score = (0..7)
            .filter(|&vertex| reference.active[vertex])
            .map(|vertex| {
                reference.fill_count_of_bs(vertex as u32) as i64
                    + i64::from(degree_coefficient) * reference.degree(vertex as u32) as i64
            })
            .min()
            .unwrap();
        assert_eq!(
            selected_score, minimum_score,
            "step {step} selected vertex {selected} with score {selected_score}, minimum {minimum_score}",
        );
        reference.eliminate(selected);
    }
}

#[test]
fn sampled_fill_degree_selects_a_minimum_score() {
    for degree_coefficient in [1, -1, -2, -16] {
        assert_sampled_fill_degree_minimizes_score(degree_coefficient);
    }
}

/// Four hubs and `leaves` vertices of degree three, each leaf joined to three
/// of the four hubs. Scoring one leaf reads all three hub rows, so a vertex of
/// degree three costs as much as tens of thousands of ordinary ones — the shape
/// an incidence graph's clause and variable vertices have.
fn hub_graph(leaves: u32) -> (u32, Vec<(u32, u32)>) {
    let hubs = 4;
    let edges = (0..leaves)
        .flat_map(|leaf| {
            let vertex = hubs + leaf;
            let skipped = leaf % hubs;
            (0..hubs)
                .filter(move |hub| *hub != skipped)
                .map(move |hub| (hub, vertex))
        })
        .collect();
    (hubs + leaves, edges)
}

#[test]
fn the_seeding_scan_stops_within_a_millisecond_of_the_hard_deadline() {
    let (n, edges) = hub_graph(20_000);
    let mut graph = EliminationGraph::from_edges(n, &edges);
    let salt = vec![0u32; n as usize];
    let mut bags = Vec::new();
    let mut rank = Vec::new();
    let sink = ElimSink::new(&mut bags, &mut rank, 0);

    let epoch = std::time::Instant::now();
    let _meter = crate::meter::arm(epoch);
    let deadline = epoch + std::time::Duration::from_millis(1);
    let exit = eliminate_min_fill(
        &mut graph,
        &salt,
        sink,
        ElimStop {
            soft_deadline: None,
            hard_deadline: Some(deadline),
            width_bound: None,
            abort_on_tie: false,
        },
    );

    assert_eq!(exit, ElimExit::DeadlineReached(Cutoff::Hard));
    let overrun = crate::meter::now().saturating_duration_since(deadline);
    assert!(
        overrun <= std::time::Duration::from_millis(1),
        "the seeding scan ran {overrun:?} past the hard deadline",
    );
}
