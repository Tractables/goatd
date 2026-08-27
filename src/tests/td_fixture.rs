//! The tree decompositions the construction tests run over, and the checks
//! every one of them makes on a decomposition.
//!
//! A decomposition is valid or not on the same terms whichever backend
//! produced it, and one implementation of that judgement is what keeps two
//! backends from being held to different standards.

use crate::{Graph, TdBag, TreeDecomposition};

/// A named graph a decomposition test runs over: how many vertices it has, and
/// which pairs of them are joined.
pub(crate) type NamedGraph = (&'static str, u32, &'static [(u32, u32)]);

/// A named graph carrying its own treewidth, for the tests that hold a
/// decomposition of it to that bound.
pub(crate) type GraphAtItsWidth = (&'static str, u32, &'static [(u32, u32)], u32);

/// A `TreeDecomposition` built by hand. `tree_edges` names bags by index and
/// is undirected — each edge is recorded on both sides.
pub(crate) fn make_td(bags: Vec<Vec<u32>>, tree_edges: Vec<(usize, usize)>) -> TreeDecomposition {
    let num_vertices = bags
        .iter()
        .flatten()
        .copied()
        .max()
        .map_or(0, |vertex| vertex + 1);
    make_td_for(num_vertices, bags, tree_edges)
}

/// A hand-built decomposition with an explicit vertex universe, including
/// vertices deliberately absent from its bags.
pub(crate) fn make_td_for(
    num_vertices: u32,
    bags: Vec<Vec<u32>>,
    tree_edges: Vec<(usize, usize)>,
) -> TreeDecomposition {
    let mut adj = vec![Vec::new(); bags.len()];
    for &(a, b) in &tree_edges {
        adj[a].push(b);
        adj[b].push(a);
    }
    TreeDecomposition::from_parts(
        num_vertices,
        bags.into_iter().map(TdBag::new).collect(),
        adj,
    )
}

/// Everything a tree decomposition of a graph must satisfy: every vertex is in
/// some bag, every edge's two endpoints share one, and the running intersection
/// holds.
pub(crate) fn assert_valid_td(td: &TreeDecomposition, num_vertices: u32, edges: &[(u32, u32)]) {
    let graph = Graph::new(num_vertices, edges.iter().copied());
    td.validate(&graph)
        .unwrap_or_else(|error| panic!("invalid tree decomposition: {error}"));
}

/// A three-bag path decomposition of six vertices, sharing two vertices
/// across the first join and one across the second.
pub(crate) fn make_test_td() -> TreeDecomposition {
    make_td(
        vec![vec![0, 1, 2], vec![1, 2, 3], vec![3, 4, 5]],
        vec![(0, 1), (1, 2)],
    )
}
