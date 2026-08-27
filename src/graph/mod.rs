//! The graph every decomposer here takes: an edge list over `0..num_vertices`.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::Error;

/// An undirected graph as an edge list.
///
/// [`Graph::new`] puts any edge list in canonical form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Graph {
    /// The vertices are `0..num_vertices`.
    pub(crate) num_vertices: u32,
    /// Sorted, deduplicated `(u, v)` with `u < v`, 0-indexed.
    pub(crate) edges: Vec<(u32, u32)>,
}

impl Graph {
    /// The graph over `0..num_vertices` with these edges, in either orientation
    /// and any order; self-loops are dropped.
    ///
    /// # Panics
    ///
    /// Panics if an edge endpoint is outside `0..num_vertices`. Use
    /// [`Graph::try_new`] when endpoints are not already trusted.
    pub fn new(num_vertices: u32, edges: impl IntoIterator<Item = (u32, u32)>) -> Self {
        Self::try_new(num_vertices, edges).unwrap_or_else(|error| panic!("{error}"))
    }

    /// Build a graph, returning an error for an out-of-range edge endpoint.
    /// Self-loops are dropped and repeated undirected edges are kept once.
    pub fn try_new(
        num_vertices: u32,
        edges: impl IntoIterator<Item = (u32, u32)>,
    ) -> Result<Self, Error> {
        let mut canonical = Vec::new();
        for (left, right) in edges {
            if left >= num_vertices || right >= num_vertices {
                return Err(Error::InvalidInput(format!(
                    "graph edge ({left}, {right}) has an endpoint outside 0..{num_vertices}"
                )));
            }
            canonical.push((left, right));
        }
        Ok(Graph {
            num_vertices,
            edges: canonical_edges(canonical),
        })
    }

    /// Number of vertices, with ids `0..num_vertices()`.
    pub fn num_vertices(&self) -> u32 {
        self.num_vertices
    }

    /// Canonical undirected edges, sorted and deduplicated with `u < v`.
    pub fn edges(&self) -> &[(u32, u32)] {
        &self.edges
    }

    /// The subgraph induced by `vertices`, with `vertices[i]` renumbered to
    /// local vertex `i`.
    ///
    /// # Errors
    ///
    /// Returns an error when a vertex is repeated or outside this graph.
    pub fn induced_subgraph(&self, vertices: &[u32]) -> Result<Self, Error> {
        if vertices.len() > u32::MAX as usize {
            return Err(Error::InvalidInput(format!(
                "induced subgraph has {} vertices, which does not fit in u32",
                vertices.len()
            )));
        }
        let mut seen = FxHashSet::default();
        for &vertex in vertices {
            if vertex >= self.num_vertices {
                return Err(Error::InvalidInput(format!(
                    "induced-subgraph vertex {vertex} is outside 0..{}",
                    self.num_vertices
                )));
            }
            if !seen.insert(vertex) {
                return Err(Error::InvalidInput(format!(
                    "induced-subgraph vertex {vertex} occurs more than once"
                )));
            }
        }
        Ok(Graph::new(
            vertices.len() as u32,
            induced_edges(&self.edges, vertices),
        ))
    }
}

/// Put an edge list in the form [`Graph::edges`] describes.
pub(crate) fn canonical_edges(mut edges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    edges.retain(|&(u, v)| u != v);
    for (u, v) in &mut edges {
        if *u > *v {
            std::mem::swap(u, v);
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

/// The edges of `edges` induced on `subset`, renumbered so that vertex
/// `subset[i]` becomes local id `i`, in the form [`Graph::edges`] describes.
/// A caller maps back through `subset` before touching the original ids again.
pub(crate) fn induced_edges(edges: &[(u32, u32)], subset: &[u32]) -> Vec<(u32, u32)> {
    // Charged at the length of the FULL list, because that is what the
    // restriction reads: a caller recursing over a large graph tests every
    // edge for containment at every level, so on a deep recursion this scan —
    // not the partition or the elimination that follows it — is where the
    // level's work goes.
    crate::meter::charge(edges.len() as u64);
    let local = index_by_vertex(subset);
    let mut out = Vec::new();
    for &(u, v) in edges {
        if let (Some(&lu), Some(&lv)) = (local.get(&u), local.get(&v)) {
            out.push((lu, lv));
        }
    }
    canonical_edges(out)
}

/// The local id `subset[i] -> i` that [`induced_edges`] renumbers through.
pub(crate) fn index_by_vertex(subset: &[u32]) -> FxHashMap<u32, u32> {
    subset
        .iter()
        .enumerate()
        .map(|(i, &v)| (v, i as u32))
        .collect()
}

#[cfg(test)]
mod tests;
