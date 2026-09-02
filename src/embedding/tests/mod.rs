use super::{DEFAULT_TOLERANCE, Embedding};
use crate::Graph;

/// The path `0 — 1 — … — (n-1)`.
fn path(length: u32) -> Graph {
    Graph::new(length, (0..length.saturating_sub(1)).map(|v| (v, v + 1)))
}

/// The `side × side` grid, vertex `row * side + column`.
fn grid(side: u32) -> Graph {
    let mut edges = Vec::new();
    for row in 0..side {
        for column in 0..side {
            let vertex = row * side + column;
            if column + 1 < side {
                edges.push((vertex, vertex + 1));
            }
            if row + 1 < side {
                edges.push((vertex, vertex + side));
            }
        }
    }
    Graph::new(side * side, edges)
}

/// Settle `graph` with the usual stopping rule.
fn settle(graph: &Graph, dim: usize, seed: u64, max_rounds: usize) -> Embedding {
    Embedding::compute(
        graph,
        dim,
        seed,
        max_rounds,
        5,
        DEFAULT_TOLERANCE,
        &mut || false,
    )
}

/// Every entry of the coordinate covariance, over `vertices`.
fn covariance(embedding: &Embedding, vertices: &[u32]) -> Vec<f64> {
    let count = vertices.len() as f64;
    let dim = embedding.dim();
    let mut mean = vec![0.0f64; dim];
    for &vertex in vertices {
        for (axis, value) in embedding.coord(vertex).iter().enumerate() {
            mean[axis] += f64::from(*value);
        }
    }
    for value in &mut mean {
        *value /= count;
    }
    let mut entries = vec![0.0f64; dim * dim];
    for &vertex in vertices {
        let row = embedding.coord(vertex);
        for i in 0..dim {
            for j in 0..dim {
                entries[i * dim + j] +=
                    (f64::from(row[i]) - mean[i]) * (f64::from(row[j]) - mean[j]);
            }
        }
    }
    for entry in &mut entries {
        *entry /= count;
    }
    entries
}

/// Every entry of the covariance is within `1e-3` of the identity's.
fn assert_whitened(entries: &[f64], dim: usize) {
    for i in 0..dim {
        for j in 0..dim {
            let expected = if i == j { 1.0 } else { 0.0 };
            assert!(
                (entries[i * dim + j] - expected).abs() < 1e-3,
                "covariance[{i}][{j}] is {}, expected {expected}",
                entries[i * dim + j]
            );
        }
    }
}

/// The vertices in the order their tie weights draw them.
fn weight_order(weights: &[u32]) -> Vec<u32> {
    let mut order: Vec<u32> = (0..weights.len() as u32).collect();
    order.sort_by_key(|&vertex| weights[vertex as usize]);
    order
}

#[test]
fn a_path_comes_out_monotone_along_its_only_axis() {
    // The ends of a path separate slowly, so this one runs to the round cap
    // rather than to the usual tolerance.
    let embedding = Embedding::compute(&path(50), 1, 7, 3_000, 5, 1e-12, &mut || false);

    let line: Vec<f32> = (0..50).map(|vertex| embedding.coord(vertex)[0]).collect();
    let increasing = line.windows(2).all(|pair| pair[0] < pair[1]);
    let decreasing = line.windows(2).all(|pair| pair[0] > pair[1]);
    assert!(
        increasing || decreasing,
        "the axis of a path must follow the path: {line:?}"
    );
}

#[test]
fn a_grid_puts_its_corners_farthest_from_the_centre() {
    let embedding = settle(&grid(10), 2, 1, 2_000);

    let mut by_eccentricity: Vec<u32> = (0..100).collect();
    by_eccentricity.sort_by(|&left, &right| {
        embedding
            .eccentricity(right)
            .total_cmp(&embedding.eccentricity(left))
    });
    let mut farthest = by_eccentricity[..4].to_vec();
    farthest.sort_unstable();
    assert_eq!(farthest, vec![0, 9, 90, 99]);
}

#[test]
fn whitening_leaves_axes_of_unit_variance_and_no_correlation() {
    let embedding = settle(&grid(10), 3, 1, 200);
    let every_vertex: Vec<u32> = (0..embedding.num_vertices() as u32).collect();

    assert_whitened(&covariance(&embedding, &every_vertex), embedding.dim());
}

#[test]
fn tie_weights_run_from_the_most_peripheral_vertex_to_the_most_central() {
    let embedding = settle(&grid(6), 2, 3, 500);
    let count = embedding.num_vertices();
    let peripheral_first = embedding.rank_weights(true);

    assert_eq!(peripheral_first.len(), count);
    assert_eq!(peripheral_first.iter().copied().min(), Some(0));
    assert_eq!(peripheral_first.iter().copied().max(), Some(u32::MAX));
    let eccentricities = |weights: &[u32]| {
        weight_order(weights)
            .into_iter()
            .map(|vertex| embedding.eccentricity(vertex))
            .collect::<Vec<f32>>()
    };
    assert!(
        eccentricities(&peripheral_first)
            .windows(2)
            .all(|pair| pair[0] >= pair[1]),
        "weight 0 is the most peripheral vertex"
    );
    assert!(
        eccentricities(&embedding.rank_weights(false))
            .windows(2)
            .all(|pair| pair[0] <= pair[1]),
        "the flipped sign puts the most central vertex first"
    );
}

#[test]
fn one_seed_gives_one_cloud() {
    let graph = grid(6);
    let first = settle(&graph, 3, 11, 100);
    let again = settle(&graph, 3, 11, 100);
    let other_seed = settle(&graph, 3, 12, 100);

    assert_eq!(first, again);
    assert_ne!(first, other_seed);
}

#[test]
fn the_stop_signal_ends_the_round_loop() {
    let graph = grid(6);
    let mut rounds = 0;
    let mut stop = || {
        rounds += 1;
        true
    };

    let embedding = Embedding::compute(&graph, 2, 0, 100, 100, DEFAULT_TOLERANCE, &mut stop);

    assert_eq!(rounds, 1, "the loop must not start a second round");
    assert_eq!(embedding.num_vertices(), 36);
}

#[test]
fn a_settled_cloud_stops_before_the_round_cap() {
    let graph = grid(6);
    let mut rounds = 0;
    let mut stop = || {
        rounds += 1;
        false
    };

    Embedding::compute(&graph, 2, 4, 5_000, 5, DEFAULT_TOLERANCE, &mut stop);

    assert!(
        rounds < 5_000,
        "a settled cloud must stop the loop, ran {rounds} rounds"
    );
}

#[test]
fn the_settling_test_ignores_a_rotated_frame() {
    // Four vertices on a path, and the same cloud turned a quarter turn:
    // (x, y) becomes (-y, x). Whitening can hand back a frame like this from
    // one round to the next when two eigenvalues are close.
    let coords: [f32; 8] = [0.0, 1.0, 2.0, -1.0, -0.5, 0.25, 1.5, 3.0];
    let turned: Vec<f32> = coords
        .as_chunks::<2>()
        .0
        .iter()
        .flat_map(|row| [-row[1], row[0]])
        .collect();
    let starts: [usize; 5] = [0, 1, 3, 5, 6];
    let targets: [u32; 6] = [1, 0, 2, 1, 3, 2];

    assert_eq!(
        super::largest_invariant_change(&coords, &turned, 2, &starts, &targets),
        0.0,
        "eccentricities and edge lengths do not turn with the frame",
    );
}

#[test]
fn an_isolated_vertex_takes_no_part_in_the_statistics() {
    // A 4×4 grid with one loose vertex added. The grid decides the whitening;
    // the loose vertex is never averaged and is only carried by it.
    let grid = grid(4);
    let graph = Graph::new(17, grid.edges().iter().copied());

    let embedding = settle(&graph, 2, 2, 500);

    assert_eq!(embedding.num_vertices(), 17);
    let attached: Vec<u32> = (0..16).collect();
    assert_whitened(&covariance(&embedding, &attached), embedding.dim());
    assert_eq!(embedding.rank_weights(true).len(), 17);
}
