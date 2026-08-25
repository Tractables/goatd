//! What a wall cap is allowed to change about the search it bounds.

use crate::flowcutter::{FcBudget, WallCapMode, flowcutter_td};
use crate::tests::td_fixture::assert_valid_td;
use crate::{Graph, TreeDecomposition};

/// A circulant triple structure over `n` vertices: vertex `i` forms a
/// triangle with `i+1` and `i+7`, another with `i+13` and `i+31`, and a third
/// with `i+57` and `i+101` (mod `n`).
///
/// The strides are coprime spreads rather than a local band, which keeps the
/// decomposition from collapsing to a path a single greedy pass finds
/// instantly.
fn circulant_triples(n: u32) -> Vec<[u32; 3]> {
    let mut triples = Vec::with_capacity(3 * n as usize);
    for i in 0..n {
        for (a, b) in [(1u32, 7u32), (13, 31), (57, 101)] {
            triples.push([i, (i + a) % n, (i + b) % n]);
        }
    }
    triples
}

/// The graph whose vertices are the `n` circulant vertices: each triple is a
/// triangle.
fn circulant_graph(n: u32) -> Graph {
    Graph::new(
        n,
        circulant_triples(n)
            .into_iter()
            .flat_map(|[a, b, c]| [(a, b), (a, c), (b, c)]),
    )
}

/// The bipartite graph between the `n` circulant vertices and the `3n`
/// triples, one vertex per triple: `4n` vertices in all.
fn circulant_incidence(n: u32) -> Graph {
    let triples = circulant_triples(n);
    Graph::new(
        n + triples.len() as u32,
        triples
            .iter()
            .enumerate()
            .flat_map(|(t, tri)| tri.iter().map(move |&v| (v, n + t as u32))),
    )
}

/// The bags of `td`, each sorted, so two decompositions can be compared as
/// values.
fn bag_sets(td: &TreeDecomposition) -> Vec<Vec<u32>> {
    td.bags
        .iter()
        .map(|b| {
            let mut v = b.vertices.clone();
            v.sort_unstable();
            v
        })
        .collect()
}

/// A [`WallCapMode::BoundOnly`] cap the build never reaches must produce the
/// same decomposition as no cap at all, bag for bag.
///
/// This is what lets a construction budget be enforced without changing what
/// construction does. The vendored timed entry differs from the step-budgeted
/// one in more than when it stops — it also tightens the pre-loop heuristic node
/// gates and drops the step clamp — so a cap that carried tightness with it
/// would change the tree on every instance, in service of bounding the few that
/// overrun.
///
/// The fixture is sized past the tight min-degree gate (700 circulant
/// vertices, so 2 800 incidence vertices, over the 2 000-vertex tight limit
/// and under the 50 000-vertex loose one) on purpose: below it both modes
/// agree trivially. Substituting [`WallCapMode::Tight`] here fails the
/// assertion.
#[test]
fn a_bound_only_wall_the_build_never_reaches_decomposes_exactly_as_no_wall_does() {
    let graph = circulant_incidence(700);

    let unbounded = flowcutter_td(
        &graph,
        FcBudget::Steps {
            steps: 20_000,
            iters: 4,
        },
    )
    .expect("the step-budgeted search decomposes this graph");

    // Ten minutes: a real bound, and one this build finishes far inside.
    let bounded = flowcutter_td(
        &graph,
        FcBudget::Timed {
            timeout_ms: 600_000,
            patience_ms: 0,
            iters: 4,
            steps: 20_000,
            cap_mode: WallCapMode::BoundOnly,
        },
    )
    .expect("the bound-only search decomposes this graph");

    assert_eq!(
        unbounded.treewidth(),
        bounded.treewidth(),
        "a bound-only wall changed the width found",
    );
    assert_eq!(
        bag_sets(&unbounded),
        bag_sets(&bounded),
        "a bound-only wall changed the decomposition itself",
    );
}

/// A greedy elimination pass that runs out of time is dropped whole, so the
/// decomposition still covers every vertex.
///
/// The two passes are abandoned mid-way under a tight wall, and an elimination
/// order missing its tail is not a permutation: handing one on would decompose a
/// subset of the graph and leave the rest of the vertices in no bag at all.
/// The contract is that an abandoned pass returns nothing and is skipped exactly
/// as the size gates skip it, which is what this asserts — whether or not the
/// wall happens to cut a pass short on any given run.
#[test]
fn a_wall_that_cuts_a_greedy_pass_short_still_yields_a_decomposition_of_the_whole_graph() {
    let n = 1_500;
    let graph = circulant_graph(n);

    let td = flowcutter_td(
        &graph,
        FcBudget::Timed {
            timeout_ms: 1,
            patience_ms: 0,
            iters: 4,
            steps: 20_000,
            cap_mode: WallCapMode::Tight,
        },
    )
    .expect("a wall that expires immediately still yields a decomposition");

    assert_valid_td(&td, n, &graph.edges);
}
