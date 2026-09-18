//! One pass from a [`Graph`]'s edge list to a flat neighbour array.

use crate::Graph;

/// Compressed adjacency: `targets[starts[v]..starts[v + 1]]` are `v`'s
/// neighbours.
///
/// One flat array rather than a `Vec` per vertex: the arc count per vertex is
/// known after one pass over the edges, so the rows can be placed directly, and
/// on a whole graph the per-vertex headers alone would cost more than the walk
/// that follows. Building it is a pass over the edges and about as much memory
/// as a set of coordinates takes, so a caller placing several embeddings of one
/// graph builds it once and hands it to each of them.
pub(crate) struct Adjacency {
    starts: Vec<usize>,
    targets: Vec<u32>,
}

impl Adjacency {
    /// The adjacency of `graph`.
    pub(crate) fn of(graph: &Graph) -> Self {
        let vertex_count = graph.num_vertices() as usize;
        let mut starts = vec![0usize; vertex_count + 1];
        for &(left, right) in graph.edges() {
            starts[left as usize + 1] += 1;
            starts[right as usize + 1] += 1;
        }
        for vertex in 0..vertex_count {
            starts[vertex + 1] += starts[vertex];
        }
        let mut cursor = starts[..vertex_count].to_vec();
        let mut targets = vec![0u32; graph.edges().len() * 2];
        for &(left, right) in graph.edges() {
            targets[cursor[left as usize]] = right;
            cursor[left as usize] += 1;
            targets[cursor[right as usize]] = left;
            cursor[right as usize] += 1;
        }
        Adjacency { starts, targets }
    }

    /// How many vertices the graph has.
    pub(crate) fn vertex_count(&self) -> usize {
        self.starts.len() - 1
    }

    /// `vertex`'s neighbours.
    pub(crate) fn neighbours(&self, vertex: usize) -> &[u32] {
        &self.targets[self.starts[vertex]..self.starts[vertex + 1]]
    }

    /// The two arrays, for a hot loop that indexes them itself.
    pub(crate) fn rows(&self) -> (&[usize], &[u32]) {
        (&self.starts, &self.targets)
    }
}

/// The neighbours of every vertex, in edge order, as one row per vertex.
///
/// A caller that reads a row at a time and does not mind the allocations takes
/// this; one that walks the whole graph takes [`Adjacency::of`]. Charging the
/// meter for the pass is the caller's, since only some of them run under a
/// budget.
pub(crate) fn lists(graph: &Graph) -> Vec<Vec<u32>> {
    let mut lists = vec![Vec::new(); graph.num_vertices() as usize];
    for &(left, right) in graph.edges() {
        lists[left as usize].push(right);
        lists[right as usize].push(left);
    }
    lists
}
