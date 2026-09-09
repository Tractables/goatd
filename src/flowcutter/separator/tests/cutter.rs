use crate::flowcutter::separator::*;

/// A 4x4 grid, as (n, edges).
fn grid() -> (u32, Vec<(u32, u32)>) {
    let side = 4u32;
    let mut edges = Vec::new();
    for r in 0..side {
        for c in 0..side {
            let v = r * side + c;
            if c + 1 < side {
                edges.push((v, v + 1));
            }
            if r + 1 < side {
                edges.push((v, v + side));
            }
        }
    }
    (side * side, edges)
}

fn assert_same_state(left: &MultiCutter, right: &MultiCutter, n: u32, step: usize) {
    assert_eq!(
        left.current_cut(),
        right.current_cut(),
        "cut at step {step}"
    );
    assert_eq!(
        left.current_smaller_size(),
        right.current_smaller_size(),
        "smaller side at step {step}"
    );
    for x in 0..n_exp(n) {
        assert_eq!(
            left.is_on_smaller_side(x),
            right.is_on_smaller_side(x),
            "node {x} at step {step}"
        );
    }
}

/// The search runs one cutter for all of its iterations and starts it over with
/// `init` at the top of each one, so `init` has to leave nothing of the
/// previous iteration behind.
#[test]
fn a_reused_cutter_searches_exactly_as_a_fresh_one_does() {
    let (n, edges) = grid();
    let g = OrigGraph::build(n, &edges).expect("the grid has edges");
    let a_orig = g.tail.len() as u32;
    let exp = Exp { g: &g, a_orig };

    let first = [(orig_node_to_exp(0, false), orig_node_to_exp(15, true))];
    let second = [(orig_node_to_exp(3, false), orig_node_to_exp(12, true))];

    // Run a whole search on `first`, so a reused cutter has state to carry.
    let mut reused = MultiCutter::new();
    reused.init(&exp, a_orig, &first);
    while reused.advance(&exp, a_orig) {}
    reused.init(&exp, a_orig, &second);

    let mut fresh = MultiCutter::new();
    fresh.init(&exp, a_orig, &second);

    // Compare the search on `second` step by step: state that init failed to
    // clear need not show up at once.
    for step in 0.. {
        assert_same_state(&reused, &fresh, n, step);
        let reused_advanced = reused.advance(&exp, a_orig);
        let fresh_advanced = fresh.advance(&exp, a_orig);
        assert_eq!(reused_advanced, fresh_advanced, "advance at step {step}");
        if !fresh_advanced {
            break;
        }
    }
}
