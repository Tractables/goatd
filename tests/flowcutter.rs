use std::time::{Duration, Instant};

use goatd::flowcutter::{Budget, TimeoutBehavior, decompose};
use goatd::meter::{UNITS_PER_MS, arm, units_spent};
use goatd::{Error, Graph, TreeDecomposition};

type GraphAtWidth = (&'static str, u32, &'static [(u32, u32)], u32);

#[test]
fn the_empty_graph_has_an_empty_decomposition() {
    let graph = Graph::new(0, []);
    let decomposition = decompose(&graph, Budget::steps(1, 1)).unwrap();

    assert!(decomposition.bags().is_empty());
    decomposition.validate(&graph).unwrap();
}

#[test]
fn step_budgets_repeat_on_small_graph_families() {
    let shapes: &[GraphAtWidth] = &[
        ("one edge", 2, &[(0, 1)], 1),
        ("a path", 5, &[(0, 1), (1, 2), (2, 3), (3, 4)], 1),
        ("a cycle", 4, &[(0, 1), (1, 2), (2, 3), (3, 0)], 2),
        ("a star", 4, &[(0, 1), (0, 2), (0, 3)], 1),
        (
            "a complete graph",
            4,
            &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)],
            3,
        ),
        (
            "two triangles",
            6,
            &[(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)],
            2,
        ),
    ];

    let budget = Budget::steps(10_000, 8);
    for &(shape, num_vertices, edges, width) in shapes {
        let graph = Graph::new(num_vertices, edges.iter().copied());
        let decomposition =
            decompose(&graph, budget).unwrap_or_else(|error| panic!("{shape}: {error}"));
        decomposition.validate(&graph).unwrap();
        assert!(decomposition.treewidth() <= width, "{shape}");

        let repeated = decompose(&graph, budget).unwrap_or_else(|error| panic!("{shape}: {error}"));
        assert_eq!(decomposition, repeated, "{shape}");
    }
}

#[test]
fn zero_work_budgets_are_rejected() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);
    let timed = decompose(&graph, Budget::timed(Duration::ZERO, None, 12)).unwrap_err();
    let patience = decompose(
        &graph,
        Budget::timed(Duration::from_secs(1), Some(Duration::ZERO), 12),
    )
    .unwrap_err();
    let safety_timeout = decompose(
        &graph,
        Budget::steps(10, 1).with_timeout(Duration::ZERO, None, TimeoutBehavior::StopOnly),
    )
    .unwrap_err();
    let steps = decompose(&graph, Budget::steps(0, 12)).unwrap_err();

    assert!(matches!(timed, Error::InvalidInput(_)));
    assert!(matches!(patience, Error::InvalidInput(_)));
    assert!(matches!(safety_timeout, Error::InvalidInput(_)));
    assert!(matches!(steps, Error::InvalidInput(_)));
}

#[test]
fn work_limits_that_do_not_fit_the_backend_are_rejected() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);

    assert!(decompose(&graph, Budget::steps(i64::MAX as u64 + 1, 1)).is_err());
    assert!(decompose(&graph, Budget::steps(1, i32::MAX as u32 + 1)).is_err());
    assert!(decompose(&graph, Budget::timed(Duration::MAX, None, 1)).is_err());
}

fn circulant_triples(n: u32) -> Vec<[u32; 3]> {
    let mut triples = Vec::with_capacity(3 * n as usize);
    for vertex in 0..n {
        for (left, right) in [(1, 7), (13, 31), (57, 101)] {
            triples.push([vertex, (vertex + left) % n, (vertex + right) % n]);
        }
    }
    triples
}

fn circulant_graph(n: u32) -> Graph {
    Graph::new(
        n,
        circulant_triples(n)
            .into_iter()
            .flat_map(|[a, b, c]| [(a, b), (a, c), (b, c)]),
    )
}

fn cycle_graph(n: u32) -> Graph {
    Graph::new(n, (0..n).map(|vertex| (vertex, (vertex + 1) % n)))
}

#[test]
fn metered_patience_stops_before_the_timeout_after_the_width_stalls() {
    let graph = cycle_graph(100);
    let before = units_spent();
    let result = {
        let _meter = arm(Instant::now());
        decompose(
            &graph,
            Budget::timed(
                Duration::from_millis(10),
                Some(Duration::from_millis(1)),
                1_000,
            ),
        )
    };
    let spent = units_spent() - before;

    result.unwrap().validate(&graph).unwrap();
    assert!(
        spent < 5 * UNITS_PER_MS,
        "patience spent {spent} work units"
    );
}

#[test]
fn metered_positive_submillisecond_timeout_is_not_treated_as_unmetered() {
    let graph = cycle_graph(50);
    let before = units_spent();
    let result = {
        let _meter = arm(Instant::now());
        decompose(&graph, Budget::timed(Duration::from_nanos(1), None, 100))
    };
    let spent = units_spent() - before;

    assert!(matches!(result, Err(Error::NoDecomposition)));
    assert!(
        spent < UNITS_PER_MS,
        "a one-millisecond work budget spent {spent} units"
    );
}

fn circulant_incidence(n: u32) -> Graph {
    let triples = circulant_triples(n);
    Graph::new(
        n + triples.len() as u32,
        triples.iter().enumerate().flat_map(|(triple, vertices)| {
            vertices
                .iter()
                .map(move |&vertex| (vertex, n + triple as u32))
        }),
    )
}

fn bag_sets(decomposition: &TreeDecomposition) -> Vec<Vec<u32>> {
    decomposition
        .bags()
        .iter()
        .map(|bag| {
            let mut vertices = bag.vertices().to_vec();
            vertices.sort_unstable();
            vertices
        })
        .collect()
}

#[test]
fn an_unreached_stop_only_timeout_preserves_the_step_search() {
    let graph = circulant_incidence(700);
    let unbounded = decompose(&graph, Budget::steps(20_000, 4)).unwrap();
    let bounded = decompose(
        &graph,
        Budget::steps(20_000, 4).with_timeout(
            Duration::from_secs(600),
            None,
            TimeoutBehavior::StopOnly,
        ),
    )
    .unwrap();

    assert_eq!(unbounded.treewidth(), bounded.treewidth());
    assert_eq!(bag_sets(&unbounded), bag_sets(&bounded));
}

#[test]
fn an_adaptive_timeout_still_returns_a_complete_decomposition() {
    let graph = circulant_graph(1_500);
    let decomposition = decompose(
        &graph,
        Budget::steps(20_000, 4).with_timeout(
            Duration::from_millis(1),
            None,
            TimeoutBehavior::AdaptSearch,
        ),
    )
    .unwrap();

    decomposition.validate(&graph).unwrap();
}

#[test]
fn a_standalone_budget_is_the_one_the_command_line_built() {
    let clock = Duration::from_millis(40);
    let patience = Some(Duration::from_millis(150));

    assert_eq!(Budget::standalone(None, Some(7)), Budget::steps(7, 900));
    assert_eq!(
        Budget::standalone(Some(clock), Some(7)),
        Budget::steps(7, 900),
        "a step count wins over a clock"
    );
    assert_eq!(
        Budget::standalone(Some(clock), None),
        Budget::timed(clock, patience, 100_000)
    );
    assert_eq!(
        Budget::standalone(None, None),
        Budget::timed(Duration::from_millis(200), patience, 100_000)
    );
}
