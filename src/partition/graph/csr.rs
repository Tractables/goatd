//! The CSR graph used at every level of the hierarchy, and the builder that
//! turns a local edge list into the finest one.
//!
//! Only the finest level has all-ones weights: coarsening sums the weight of
//! everything it contracts, so `vertex_weights` and `edge_weights` carry the
//! fine graph's mass upward and let the coarse levels stand in for it.

/// Vertices are 0-indexed.
pub(super) struct CsrGraph {
    /// `neighbors[offsets[v]..offsets[v + 1]]` is vertex `v`'s adjacency.
    pub(super) offsets: Vec<u32>,
    pub(super) neighbors: Vec<u32>,
    /// Number of fine vertices collapsed into this coarse vertex. Balance
    /// constraints are expressed in these units.
    pub(super) vertex_weights: Vec<u32>,
    /// Parallel to `neighbors`: total weight of the fine edges collapsed into
    /// that coarse edge. Gains and cuts are expressed in these units.
    pub(super) edge_weights: Vec<u32>,
}

impl CsrGraph {
    pub(super) fn num_vertices(&self) -> usize {
        self.offsets.len() - 1
    }

    /// Vertices plus arcs: what one pass over this graph touches, and the unit
    /// every phase of the bisection charges its passes in.
    ///
    /// The charge is taken once per pass rather than inside the loops that make
    /// one up, because several of those loops index `neighbors` directly instead of
    /// going through [`CsrGraph::neighbors`] — a charge on the accessor would
    /// miss most of the work the pass actually does.
    pub(super) fn pass_units(&self) -> u64 {
        (self.offsets.len() as u64).saturating_add(self.neighbors.len() as u64)
    }

    pub(super) fn neighbors(&self, v: usize) -> &[u32] {
        let start = self.offsets[v] as usize;
        let end = self.offsets[v + 1] as usize;
        &self.neighbors[start..end]
    }
}

/// `edges` are undirected pairs indexing into `0..n`.
///
/// Repeats collapse and every edge starts at weight 1, so multiplicity in
/// `edges` does not reach `edge_weights` — a weight above 1 only ever comes from
/// coarsening.
pub(super) fn build_csr(n: usize, edges: &[(u32, u32)]) -> CsrGraph {
    // Counted, then scattered into one buffer. A `Vec` per vertex costs n
    // allocations and n headers before any adjacency is written; the arc count
    // per vertex is known after one pass over `edges`, so the rows can be
    // placed directly and sorted where they lie.
    let mut bounds = vec![0u32; n + 1];
    for &(u, v) in edges {
        let (u, v) = (u as usize, v as usize);
        assert!(u < n && v < n, "partition edge endpoint outside 0..{n}");
        if u != v {
            bounds[u + 1] += 1;
            bounds[v + 1] += 1;
        }
    }
    for v in 0..n {
        bounds[v + 1] += bounds[v];
    }
    let mut scattered = vec![0u32; bounds[n] as usize];
    let mut cursor = bounds[..n].to_vec();
    for &(u, v) in edges {
        if u != v {
            scattered[cursor[u as usize] as usize] = v;
            cursor[u as usize] += 1;
            scattered[cursor[v as usize] as usize] = u;
            cursor[v as usize] += 1;
        }
    }

    // Repeats collapse here, so a row can end shorter than it was counted and
    // the offsets are rewritten as each row is copied out in vertex order.
    let mut offsets = Vec::with_capacity(n + 1);
    let mut neighbors = Vec::with_capacity(scattered.len());
    offsets.push(0u32);
    for v in 0..n {
        let row = &mut scattered[bounds[v] as usize..bounds[v + 1] as usize];
        row.sort_unstable();
        let mut previous = None;
        for &neighbor in row.iter() {
            if previous != Some(neighbor) {
                neighbors.push(neighbor);
                previous = Some(neighbor);
            }
        }
        offsets.push(neighbors.len() as u32);
    }
    let edge_weights = vec![1u32; neighbors.len()];
    let vertex_weights = vec![1u32; n];
    CsrGraph {
        offsets,
        neighbors,
        vertex_weights,
        edge_weights,
    }
}
