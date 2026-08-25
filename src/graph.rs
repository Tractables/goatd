//! The graph every decomposer here takes: an edge list over `0..num_vertices`.

use rustc_hash::FxHashMap;

/// An undirected graph as an edge list.
///
/// [`Graph::new`] puts any edge list in the canonical form the field
/// describes; a caller building the struct directly is responsible for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Graph {
    /// The vertices are `0..num_vertices`.
    pub num_vertices: u32,
    /// Sorted, deduplicated `(u, v)` with `u < v`, 0-indexed.
    pub edges: Vec<(u32, u32)>,
}

impl Graph {
    /// The graph over `0..num_vertices` with these edges, in either orientation
    /// and any order; self-loops are dropped.
    pub fn new(num_vertices: u32, edges: impl IntoIterator<Item = (u32, u32)>) -> Self {
        let edges = edges
            .into_iter()
            .filter(|&(u, v)| u != v)
            .map(|(u, v)| (u.min(v), u.max(v)))
            .collect();
        Graph {
            num_vertices,
            edges: canonical(edges),
        }
    }
}

/// Put an edge list in the form [`Graph::edges`] describes.
pub(crate) fn canonical(mut edges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    edges.sort_unstable();
    edges.dedup();
    edges
}

/// The edges of `edges` induced on `subset`, renumbered so that vertex
/// `subset[i]` becomes local id `i`, in the form [`Graph::edges`] describes.
/// A caller maps back through `subset` before touching the original ids again.
pub fn restrict_to_subset(edges: &[(u32, u32)], subset: &[u32]) -> Vec<(u32, u32)> {
    // Charged at the length of the FULL list, because that is what the
    // restriction reads: a caller recursing over a large graph tests every
    // edge for containment at every level, so on a deep recursion this scan —
    // not the partition or the elimination that follows it — is where the
    // level's work goes.
    crate::meter::charge(edges.len() as u64);
    let local = local_index(subset);
    let mut out = Vec::new();
    for &(u, v) in edges {
        if let (Some(&lu), Some(&lv)) = (local.get(&u), local.get(&v)) {
            out.push((lu.min(lv), lu.max(lv)));
        }
    }
    canonical(out)
}

/// The local id `subset[i] -> i` that [`restrict_to_subset`] renumbers through.
pub fn local_index(subset: &[u32]) -> FxHashMap<u32, u32> {
    subset
        .iter()
        .enumerate()
        .map(|(i, &v)| (v, i as u32))
        .collect()
}
