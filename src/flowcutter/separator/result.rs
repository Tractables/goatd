//! The top-level balanced separator, packaged for a caller that wants a split
//! rather than a decomposition.
//!
//! The search returns the separator alone; this recovers the two remaining
//! sides by flood-fill on `G \ S`.

/// Result of a single FlowCutter top-level separator computation.
///
/// `side_a` and `side_b` are disjoint from `separator` and from each other.
/// Their union plus `separator` covers every vertex in the input subgraph.
/// When `G \ S` has more than two connected components, components are packed
/// greedily into the smaller side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Separator {
    /// The balanced separator: vertices removed to split the graph into `side_a`/`side_b`.
    separator: Vec<u32>,
    /// One side of the graph with `separator` removed.
    side_a: Vec<u32>,
    /// The other side of the graph with `separator` removed.
    side_b: Vec<u32>,
}

impl Separator {
    /// Vertices in the separator.
    pub fn vertices(&self) -> &[u32] {
        &self.separator
    }

    /// One side after the separator is removed.
    pub fn side_a(&self) -> &[u32] {
        &self.side_a
    }

    /// The other side after the separator is removed.
    pub fn side_b(&self) -> &[u32] {
        &self.side_b
    }

    /// Consume the result as `(separator, side_a, side_b)`.
    pub fn into_parts(self) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
        (self.separator, self.side_a, self.side_b)
    }
}

/// Add the two sides of `separator` in `graph`.
pub(crate) fn with_sides(graph: &crate::Graph, separator: Vec<u32>) -> Option<Separator> {
    let num_nodes = graph.num_vertices as usize;
    if num_nodes < 3 {
        return None;
    }

    if separator.is_empty() || separator.len() >= num_nodes {
        return None;
    }

    let (side_a, side_b) = split_sides_bfs(num_nodes, &graph.edges, &separator)?;
    Some(Separator {
        separator,
        side_a,
        side_b,
    })
}

/// Flood-fill G \ S into connected components and pack components greedily
/// into two sides balanced by vertex count.  Returns `None` if either side
/// ends up empty.
fn split_sides_bfs(
    num_nodes: usize,
    edges: &[(u32, u32)],
    separator: &[u32],
) -> Option<(Vec<u32>, Vec<u32>)> {
    let in_sep = {
        let mut flag = vec![false; num_nodes];
        for &v in separator {
            flag[v as usize] = true;
        }
        flag
    };

    // Counted, then scattered into one buffer, the shape `build_csr` uses: a
    // `Vec` per vertex costs `num_nodes` allocations and `num_nodes` headers
    // before any adjacency is written, and this runs once per candidate the
    // refinement session offers. No sort and no dedup, unlike `build_csr`:
    // `edges` is canonical already, and each row ends up holding what the
    // per-vertex vector held, in the same order.
    debug_assert!(
        edges.len() <= u32::MAX as usize / 2,
        "the arc count has to fit the u32 offsets"
    );
    let mut offsets = vec![0u32; num_nodes + 1];
    for &(u, v) in edges {
        if in_sep[u as usize] || in_sep[v as usize] {
            continue;
        }
        offsets[u as usize + 1] += 1;
        offsets[v as usize + 1] += 1;
    }
    for v in 0..num_nodes {
        offsets[v + 1] += offsets[v];
    }
    let mut neighbors = vec![0u32; offsets[num_nodes] as usize];
    let mut cursor = offsets[..num_nodes].to_vec();
    for &(u, v) in edges {
        if in_sep[u as usize] || in_sep[v as usize] {
            continue;
        }
        neighbors[cursor[u as usize] as usize] = v;
        cursor[u as usize] += 1;
        neighbors[cursor[v as usize] as usize] = u;
        cursor[v as usize] += 1;
    }

    let mut component_of = vec![u32::MAX; num_nodes];
    let mut components: Vec<Vec<u32>> = Vec::new();

    for start in 0..num_nodes {
        if in_sep[start] || component_of[start] != u32::MAX {
            continue;
        }
        let cid = components.len() as u32;
        let mut stack = vec![start as u32];
        let mut comp = Vec::new();
        component_of[start] = cid;
        while let Some(v) = stack.pop() {
            comp.push(v);
            let row_start = offsets[v as usize] as usize;
            let row_end = offsets[v as usize + 1] as usize;
            for &nb in &neighbors[row_start..row_end] {
                if component_of[nb as usize] == u32::MAX {
                    component_of[nb as usize] = cid;
                    stack.push(nb);
                }
            }
        }
        components.push(comp);
    }

    if components.is_empty() {
        return None;
    }

    // Pack components into two sides greedily (largest-first) to balance.
    components.sort_by_key(|c| std::cmp::Reverse(c.len()));
    let mut side_a: Vec<u32> = Vec::new();
    let mut side_b: Vec<u32> = Vec::new();
    for comp in components {
        if side_a.len() <= side_b.len() {
            side_a.extend(comp);
        } else {
            side_b.extend(comp);
        }
    }

    if side_a.is_empty() || side_b.is_empty() {
        return None;
    }

    side_a.sort_unstable();
    side_b.sort_unstable();
    Some((side_a, side_b))
}
