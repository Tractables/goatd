use super::{Xorshift64, fmix64, restart_seed};
use crate::portfolio::SECOND_CANDIDATE_SEED_OFFSET;

#[test]
fn equal_rng_states_produce_equal_streams() {
    let draw = || {
        let mut rng = Xorshift64::from_state(0x1234_5678_9abc_def0);
        (0..16).map(|_| rng.next_u64()).collect::<Vec<_>>()
    };

    assert_eq!(draw(), draw());
}

#[test]
fn next_u32_is_the_low_half_of_the_same_next_u64_draw() {
    let mut wide = Xorshift64::from_state(17);
    let mut narrow = Xorshift64::from_state(17);

    assert_eq!(narrow.next_u32(), wide.next_u64() as u32);
}

#[test]
fn zero_is_the_documented_fixed_point() {
    let mut rng = Xorshift64::from_state(0);

    assert!((0..8).all(|_| rng.next_u64() == 0));
}

#[test]
fn zero_base_seed_uses_the_restart_sequence() {
    assert_eq!(fmix64(0), 0, "the fixed point the invariant rests on");
    for restart in 0..8 {
        let expected = (restart as u64) * 7919 + 42;
        assert_eq!(restart_seed(0, restart), expected, "restart {restart}");
    }
}

#[test]
fn portfolio_candidate_seeds_land_on_different_streams() {
    for base in [1u64, 7, 42, 12_345, u64::MAX / 3] {
        for restart in 0..8 {
            assert_ne!(
                restart_seed(base, restart),
                restart_seed(base.wrapping_add(SECOND_CANDIDATE_SEED_OFFSET), restart),
                "base {base}, restart {restart}",
            );
        }
    }
}

#[test]
fn nonzero_base_seed_changes_every_restart_stream() {
    for base in [1u64, 7, 42, 12_345] {
        for restart in 0..8 {
            assert_ne!(
                restart_seed(base, restart),
                restart_seed(0, restart),
                "base {base}, restart {restart}",
            );
        }
    }
}

// The portfolio hands its two nested-dissection candidates the seed pair
// `(s, s + 42)`. Exercise that pair through both partitioners.

const SEED_A: u64 = 12_345;
const SEED_B: u64 = SEED_A + SECOND_CANDIDATE_SEED_OFFSET;

fn random_graph(n: usize, m: usize) -> Vec<(u32, u32)> {
    let mut rng = Xorshift64::from_state(0x2545_f491_4f6c_dd1d);
    let mut edges: Vec<(u32, u32)> = Vec::with_capacity(m + n);
    for vertex in 0..n {
        edges.push((vertex as u32, ((vertex + 1) % n) as u32));
    }
    while edges.len() < m {
        let u = (rng.next_u64() as usize) % n;
        let v = (rng.next_u64() as usize) % n;
        if u != v {
            edges.push((u.min(v) as u32, u.max(v) as u32));
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

fn assert_proper_bisection(part: &[u8], n: usize) {
    assert_eq!(part.len(), n, "partition covers every vertex");
    assert!(part.contains(&0), "side 0 non-empty");
    assert!(part.contains(&1), "side 1 non-empty");
}

fn graph_part(seed: u64, edges: &[(u32, u32)], n: usize) -> Vec<u8> {
    let graph = crate::Graph::new(n as u32, edges.iter().copied());
    crate::partition::multilevel_graph_bisect(
        &graph,
        crate::partition::GraphBisectionConfig::new(0.2, seed),
    )
    .unwrap()
    .into_parts()
}

fn hypergraph_part(seed: u64, hyperedges: &[Vec<u32>], n: usize) -> Vec<u8> {
    let hypergraph = crate::partition::Hypergraph::new(n as u32, hyperedges, None).unwrap();
    crate::partition::multilevel_hypergraph_bisect(
        &hypergraph,
        crate::partition::HypergraphBisectionConfig::new(0.03, seed),
    )
    .unwrap()
    .into_parts()
}

#[test]
fn graph_bisect_differs_across_portfolio_seeds() {
    let n = 200;
    let edges = random_graph(n, 800);
    let a = graph_part(SEED_A, &edges, n);
    let b = graph_part(SEED_B, &edges, n);
    assert_proper_bisection(&a, n);
    assert_proper_bisection(&b, n);
    assert_ne!(a, b, "distinct seeds must give distinct partitions");
    assert_eq!(a, graph_part(SEED_A, &edges, n), "deterministic per seed");
}

#[test]
fn hypergraph_bisect_differs_across_portfolio_seeds() {
    let n = 120;
    let hyperedges: Vec<Vec<u32>> = random_graph(n, 480)
        .into_iter()
        .map(|(u, v)| vec![u, v])
        .collect();
    let a = hypergraph_part(SEED_A, &hyperedges, n);
    let b = hypergraph_part(SEED_B, &hyperedges, n);
    assert_proper_bisection(&a, n);
    assert_proper_bisection(&b, n);
    assert_ne!(a, b, "distinct seeds must give distinct partitions");
    assert_eq!(
        a,
        hypergraph_part(SEED_A, &hyperedges, n),
        "deterministic per seed"
    );
}
