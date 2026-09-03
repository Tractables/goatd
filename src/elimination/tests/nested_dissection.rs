use crate::elimination::nested_dissection::*;

/// The parameters every test here runs under, varying only the base-case
/// threshold: no deadline, seed 0, and the balance tolerance the production
/// caller passes.
fn params<'a>(salt: &'a [u32], base_case_size: usize) -> NestedDissectionParams<'a> {
    NestedDissectionParams {
        salt,
        base_case_size,
        max_imbalance: 0.2,
        hard_deadline: None,
        base_seed: 0,
    }
}

#[test]
fn empty_graph_returns_empty_order() {
    let order = nested_dissection_order(&[], &[], &params(&[], 32), 0);
    assert!(order.is_empty());
}

#[test]
fn small_graph_falls_through_to_base_case() {
    let active = vec![0, 1, 2];
    let edges = vec![(0, 1), (0, 2), (1, 2)];
    let salt = vec![7, 3, 11];
    let order = nested_dissection_order(&active, &edges, &params(&salt, 32), 0);
    assert_eq!(order.len(), 3);
    let mut s: Vec<u32> = order.clone();
    s.sort();
    assert_eq!(s, vec![0, 1, 2]);
}

#[test]
fn grid_10x10_produces_full_order() {
    let mut edges = Vec::new();
    for r in 0..10u32 {
        for c in 0..10u32 {
            let v = r * 10 + c;
            if c + 1 < 10 {
                edges.push((v, v + 1));
            }
            if r + 1 < 10 {
                edges.push((v, v + 10));
            }
        }
    }
    let active: Vec<u32> = (0..100).collect();
    let salt: Vec<u32> = (0..100)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761))
        .collect();
    let order = nested_dissection_order(&active, &edges, &params(&salt, 8), 0);
    assert_eq!(order.len(), 100);
    let mut s = order.clone();
    s.sort();
    assert_eq!(s, (0..100).collect::<Vec<u32>>());
}

#[test]
fn the_base_case_stops_at_the_hard_deadline_and_still_returns_a_full_order() {
    // The complete graph on 1,200 vertices minus a perfect matching goes
    // straight to the base case when the threshold is at least its size, the
    // way a level whose bisection came out degenerate does. It is dense but
    // not a clique, so min-fill has to score every vertex before its first
    // elimination, and that scan alone charges the meter tens of
    // milliseconds; a deadline one millisecond of work away stops the base
    // case part way. The order must still be a permutation, and the base
    // case must not have run on past the deadline. The meter is armed, so
    // this counts work and is not a race with the wall.
    let n = 1200u32;
    let active: Vec<u32> = (0..n).collect();
    let mut edges = Vec::new();
    for u in 0..n {
        for v in u + 1..n {
            if v != u + n / 2 {
                edges.push((u, v));
            }
        }
    }
    let salt: Vec<u32> = (0..n).map(|i| i.wrapping_mul(2_654_435_761)).collect();

    let epoch = std::time::Instant::now();
    let _meter = crate::meter::arm(epoch);
    let deadline = epoch + std::time::Duration::from_millis(1);
    let mut params = params(&salt, n as usize);
    params.hard_deadline = Some(deadline);

    let order = nested_dissection_order(&active, &edges, &params, 0);

    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(sorted, active);
    let overrun = crate::meter::now().saturating_duration_since(deadline);
    assert!(
        overrun <= std::time::Duration::from_millis(2),
        "the base case ran {overrun:?} past the hard deadline"
    );
}
