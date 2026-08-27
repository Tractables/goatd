use goatd::Graph;
use goatd::partition::{
    GraphBisectionConfig, Hypergraph, HypergraphBisectionConfig, multilevel_graph_bisect,
    multilevel_hypergraph_bisect,
};

type NamedGraph = (&'static str, u32, &'static [(u32, u32)]);

fn graph_config(seed: u64) -> GraphBisectionConfig {
    GraphBisectionConfig::new(0.2, seed)
}

fn hypergraph_config(seed: u64) -> HypergraphBisectionConfig {
    HypergraphBisectionConfig::new(0.2, seed)
}

fn assert_two_nonempty_sides(part: &[u8], n: usize) {
    assert_eq!(part.len(), n);
    assert!(part.iter().all(|&side| side <= 1));
    if n >= 2 {
        assert!(part.contains(&0));
        assert!(part.contains(&1));
    }
}

fn assert_balanced(part: &[u8], max_imbalance: f64) {
    let max_side = ((part.len() as f64) * (0.5 + max_imbalance)).ceil() as usize;
    let side_zero = part.iter().filter(|&&side| side == 0).count();
    assert!(side_zero <= max_side);
    assert!(part.len() - side_zero <= max_side);
}

#[test]
fn graph_bisection_covers_the_small_graph_families() {
    let shapes: &[NamedGraph] = &[
        ("no vertices", 0, &[]),
        ("one vertex", 1, &[]),
        ("two vertices", 2, &[(0, 1)]),
        ("a path", 6, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)]),
        ("a star", 5, &[(0, 1), (0, 2), (0, 3), (0, 4)]),
        (
            "a complete graph",
            5,
            &[
                (0, 1),
                (0, 2),
                (0, 3),
                (0, 4),
                (1, 2),
                (1, 3),
                (1, 4),
                (2, 3),
                (2, 4),
                (3, 4),
            ],
        ),
        ("no edges", 5, &[]),
        (
            "two triangles",
            6,
            &[(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)],
        ),
    ];

    for &(shape, num_vertices, edges) in shapes {
        let graph = Graph::new(num_vertices, edges.iter().copied());
        let config = GraphBisectionConfig::new(0.4, 0);
        let bisection = multilevel_graph_bisect(&graph, config).expect("valid config");
        assert_two_nonempty_sides(bisection.parts(), num_vertices as usize);
        assert_eq!(
            bisection,
            multilevel_graph_bisect(&graph, config).expect("valid config"),
            "{shape}: one seed must give one bisection",
        );
    }
}

#[test]
fn both_public_bisectors_handle_the_three_tiny_vertex_counts() {
    for n in 0..=2 {
        let graph = Graph::new(n as u32, []);
        let hypergraph = Hypergraph::new(n as u32, &[], None).unwrap();
        assert_eq!(
            multilevel_graph_bisect(&graph, graph_config(7))
                .unwrap()
                .into_parts(),
            &vec![0, 1][..n]
        );
        assert_eq!(
            multilevel_hypergraph_bisect(&hypergraph, hypergraph_config(7))
                .unwrap()
                .into_parts(),
            &vec![0, 1][..n],
        );
    }
}

#[test]
fn an_edgeless_graph_and_hypergraph_still_split_both_sides() {
    let graph = Graph::new(7, []);
    let hypergraph = Hypergraph::new(7, &[], None).unwrap();
    let graph_part = multilevel_graph_bisect(&graph, graph_config(3)).unwrap();
    let hypergraph_part = multilevel_hypergraph_bisect(&hypergraph, hypergraph_config(3)).unwrap();

    assert_two_nonempty_sides(graph_part.parts(), 7);
    assert_two_nonempty_sides(hypergraph_part.parts(), 7);
}

#[test]
fn the_public_bisectors_repeat_for_one_seed() {
    let edges = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 4),
        (4, 5),
        (5, 0),
        (0, 3),
        (1, 4),
        (2, 5),
    ];
    let hyperedges = vec![vec![0, 1, 2], vec![2, 3, 4], vec![0, 4, 5]];
    let weights = [2, 1, 3];

    let graph_input = Graph::new(6, edges);
    let hypergraph_input = Hypergraph::new(6, &hyperedges, Some(&weights)).unwrap();
    let graph = multilevel_graph_bisect(&graph_input, graph_config(99)).unwrap();
    let hypergraph =
        multilevel_hypergraph_bisect(&hypergraph_input, hypergraph_config(99)).unwrap();
    assert_eq!(
        graph.parts(),
        multilevel_graph_bisect(&graph_input, graph_config(99))
            .unwrap()
            .parts()
    );
    assert_eq!(
        hypergraph.parts(),
        multilevel_hypergraph_bisect(&hypergraph_input, hypergraph_config(99))
            .unwrap()
            .parts(),
    );
    assert_two_nonempty_sides(graph.parts(), 6);
    assert_two_nonempty_sides(hypergraph.parts(), 6);
}

#[test]
fn a_hypergraph_exposes_its_canonical_edges() {
    let hypergraph = Hypergraph::new(
        4,
        &[vec![2, 0], vec![3], vec![1, 2], vec![2, 1]],
        Some(&[3, 11, 5, 7]),
    )
    .unwrap();

    assert_eq!(hypergraph.num_vertices(), 4);
    assert_eq!(hypergraph.num_hyperedges(), 2);
    assert_eq!(
        hypergraph
            .hyperedges()
            .map(|(pins, weight)| (pins.to_vec(), weight))
            .collect::<Vec<_>>(),
        [(vec![0, 2], 3), (vec![1, 2], 12)],
    );
}

#[test]
fn both_public_bisectors_respect_exact_balance_across_seeds() {
    let num_vertices = 17;
    let edges: Vec<(u32, u32)> = (0..num_vertices)
        .flat_map(|vertex| {
            [
                (vertex, (vertex + 1) % num_vertices),
                (vertex, (vertex + 5) % num_vertices),
            ]
        })
        .collect();
    let hyperedges: Vec<Vec<u32>> = (0..num_vertices)
        .map(|vertex| {
            vec![
                vertex,
                (vertex + 1) % num_vertices,
                (vertex + 5) % num_vertices,
            ]
        })
        .collect();
    let graph = Graph::new(num_vertices, edges);
    let hypergraph = Hypergraph::new(num_vertices, &hyperedges, None).unwrap();

    for seed in 0..10 {
        let graph_part =
            multilevel_graph_bisect(&graph, GraphBisectionConfig::new(0.0, seed)).unwrap();
        let hypergraph_part =
            multilevel_hypergraph_bisect(&hypergraph, HypergraphBisectionConfig::new(0.0, seed))
                .unwrap();
        assert_balanced(graph_part.parts(), 0.0);
        assert_balanced(hypergraph_part.parts(), 0.0);
    }
}

#[test]
fn hypergraph_construction_rejects_malformed_pins_and_weights() {
    assert!(Hypergraph::new(3, &[vec![]], None).is_err());
    assert!(Hypergraph::new(3, &[vec![0, 3]], None).is_err());
    assert!(Hypergraph::new(3, &[vec![1, 1]], None).is_err());
    assert!(Hypergraph::new(3, &[vec![0, 1]], Some(&[])).is_err());
    assert!(Hypergraph::new(3, &[vec![0, 1]], Some(&[0])).is_err());
    assert!(Hypergraph::new(3, &[vec![0, 1], vec![1, 2]], Some(&[u32::MAX, 1]),).is_err());
}

#[test]
fn hypergraph_construction_canonicalizes_its_set_representation() {
    let canonical = Hypergraph::new(4, &[vec![0, 2], vec![1, 3]], Some(&[3, 4])).unwrap();
    let reordered = Hypergraph::new(
        4,
        &[vec![3, 1], vec![2], vec![2, 0], vec![0, 2]],
        Some(&[4, 99, 1, 2]),
    )
    .unwrap();

    assert_eq!(reordered, canonical);
    assert_eq!(reordered.num_hyperedges(), 2);
}

#[test]
fn hypergraph_bisection_handles_the_largest_supported_total_weight() {
    let hyperedges: Vec<Vec<u32>> = (0..20)
        .map(|vertex| vec![vertex, (vertex + 1) % 20])
        .collect();
    let mut weights = vec![1; hyperedges.len()];
    weights[0] = u32::MAX - (weights.len() as u32 - 1);
    let hypergraph = Hypergraph::new(20, &hyperedges, Some(&weights)).unwrap();

    let bisection = multilevel_hypergraph_bisect(&hypergraph, hypergraph_config(11)).unwrap();

    assert_two_nonempty_sides(bisection.parts(), 20);
}

#[test]
fn a_positive_hypergraph_effort_always_runs_one_construction() {
    let hypergraph = Hypergraph::new(3, &[vec![0, 1], vec![1, 2]], None).unwrap();
    let bisection = multilevel_hypergraph_bisect(
        &hypergraph,
        HypergraphBisectionConfig::new(0.5, 0).with_effort(f64::MIN_POSITIVE),
    )
    .unwrap();

    assert_two_nonempty_sides(bisection.parts(), 3);
}

#[test]
fn bisection_configs_reject_out_of_range_values() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);
    let hypergraph = Hypergraph::new(3, &[vec![0, 1], vec![1, 2]], None).unwrap();

    assert!(multilevel_graph_bisect(&graph, GraphBisectionConfig::new(f64::NAN, 0),).is_err());
    assert!(
        multilevel_hypergraph_bisect(
            &hypergraph,
            HypergraphBisectionConfig::new(0.1, 0).with_effort(0.0),
        )
        .is_err()
    );
    assert!(
        multilevel_hypergraph_bisect(
            &hypergraph,
            HypergraphBisectionConfig::new(0.1, 0).with_effort(101.0),
        )
        .is_err()
    );
}
