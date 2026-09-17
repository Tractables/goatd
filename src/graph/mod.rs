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
        let mut local = LocalIds::new(self.num_vertices as usize);
        // `induced_edges` already returns what [`Graph::edges`] describes: the
        // local ids are below `vertices.len()`, an injective relabelling
        // leaves no self-loop, and the list comes out sorted and deduplicated.
        // `Graph::new` would copy it and sort it a second time.
        Ok(Graph {
            num_vertices: vertices.len() as u32,
            edges: induced_edges(&self.edges, vertices, &mut local),
        })
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
///
/// `local` is the scratch index the renumbering runs through; it comes back
/// with no vertex marked, and every endpoint of `edges` must be inside it.
pub(crate) fn induced_edges(
    edges: &[(u32, u32)],
    subset: &[u32],
    local: &mut LocalIds,
) -> Vec<(u32, u32)> {
    // Charged at the length of the FULL list, because that is what the
    // restriction reads: a caller recursing over a large graph tests every
    // edge for containment at every level, so on a deep recursion this scan —
    // not the partition or the elimination that follows it — is where the
    // level's work goes.
    crate::meter::charge(edges.len() as u64);
    let out: Vec<(u32, u32)> = local.marking(subset, |position| {
        edges
            .iter()
            .filter_map(|&(u, v)| {
                let (lu, lv) = (position[u as usize], position[v as usize]);
                (lu != NOT_IN_SET && lv != NOT_IN_SET).then_some((lu, lv))
            })
            .collect()
    });
    canonical_edges(out)
}

/// The map `subset[i] -> i`, for a caller that keeps the map itself rather
/// than renumbering one list through [`LocalIds`].
pub(crate) fn index_by_vertex(subset: &[u32]) -> FxHashMap<u32, u32> {
    subset
        .iter()
        .enumerate()
        .map(|(i, &v)| (v, i as u32))
        .collect()
}

/// The entry for a vertex that is not in the set currently marked.
pub(crate) const NOT_IN_SET: u32 = u32::MAX;

/// One array over all of a graph's vertices, holding each vertex's position in
/// the set a caller is working on.
///
/// Nested dissection renumbers the same edge list three times per level — once
/// for the bisector and once for each side it recurses on — and the refinement
/// renumbers the whole graph's edge list once per region, so each lookup is one
/// array read rather than a hash. Marking a set and clearing it again are both
/// linear in the set, so the array itself is never scanned.
pub(crate) struct LocalIds {
    position: Vec<u32>,
}

impl LocalIds {
    /// Room for the vertex ids `0..vertices`, nothing marked.
    pub(crate) fn new(vertices: usize) -> Self {
        Self {
            position: vec![NOT_IN_SET; vertices],
        }
    }

    /// Run `body` with `set[i]` marked at `i` and every other vertex left at
    /// [`NOT_IN_SET`], then clear the marks again. `set` holds each vertex once.
    pub(crate) fn marking<T>(&mut self, set: &[u32], body: impl FnOnce(&[u32]) -> T) -> T {
        for (position, &vertex) in set.iter().enumerate() {
            self.position[vertex as usize] = position as u32;
        }
        let result = body(&self.position);
        for &vertex in set {
            self.position[vertex as usize] = NOT_IN_SET;
        }
        result
    }
}

#[cfg(test)]
mod tests;
