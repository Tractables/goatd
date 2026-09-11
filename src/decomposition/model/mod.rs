//! Tree-decomposition data and validation.

use rustc_hash::FxHashSet;

use crate::{Error, Graph};

/// One bag of a tree decomposition.
#[derive(Clone, Debug)]
pub struct TdBag {
    /// The bag's vertices, 0-indexed (PACE `.td` vertex ids minus one).
    pub(crate) vertices: Vec<u32>,
    /// Whether `vertices` is known to be in non-decreasing order. Recorded
    /// where the bag is built so that a subset test over hundreds of thousands
    /// of bags does not have to rediscover it. `false` means the order was
    /// never established, not that the bag is out of order.
    sorted: bool,
}

impl TdBag {
    pub(crate) fn new(mut vertices: Vec<u32>) -> Self {
        vertices.sort_unstable();
        Self {
            vertices,
            sorted: true,
        }
    }

    /// A bag emitted by an algorithm whose stable vertex order is part of its
    /// traversal result. Public constructors still enter through [`Self::new`]
    /// and canonicalize arbitrary caller input.
    pub(crate) fn from_algorithm_order(vertices: Vec<u32>) -> Self {
        let sorted = vertices.windows(2).all(|pair| pair[0] <= pair[1]);
        Self { vertices, sorted }
    }

    /// Whether the vertices are in non-decreasing order.
    pub(crate) fn is_sorted(&self) -> bool {
        self.sorted
    }

    /// Add a vertex without regard for order.
    pub(crate) fn push_unordered(&mut self, vertex: u32) {
        self.vertices.push(vertex);
        self.sorted = false;
    }

    /// Put the vertices in ascending order and drop repeats.
    pub(crate) fn sort_dedup(&mut self) {
        self.vertices.sort_unstable();
        self.vertices.dedup();
        self.sorted = true;
    }

    /// Vertices in this bag. Publicly constructed decompositions expose them in
    /// ascending order; algorithm results may retain a stable algorithm-defined
    /// order.
    pub fn vertices(&self) -> &[u32] {
        &self.vertices
    }
}

/// Two bags are the same bag when they hold the same vertices in the same
/// order. The order flag is a note about how the bag was built, not part of
/// what it holds, and it may be `false` on a bag that happens to be sorted.
impl PartialEq for TdBag {
    fn eq(&self, other: &Self) -> bool {
        self.vertices == other.vertices
    }
}

impl Eq for TdBag {}

/// A tree decomposition: bags of vertices, and an acyclic adjacency over them.
///
/// [`TreeDecomposition::new`] validates the full graph contract. PACE text can
/// be parsed before its input graph is available, so call
/// [`TreeDecomposition::validate`] on a value returned by
/// [`TreeDecomposition::from_td`]. A disconnected graph may have one bag-tree
/// component per graph component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeDecomposition {
    /// Number of vertices in the graph this decomposition was built for.
    pub(crate) num_vertices: u32,
    /// Bags, indexed by position.
    pub(crate) bags: Vec<TdBag>,
    /// Acyclic adjacency between bags, indexed like `bags`: `adj[i]` lists the
    /// bag indices connected to bag `i`.
    pub(crate) adj: Vec<Vec<usize>>,
}

impl TreeDecomposition {
    /// Build and validate a tree decomposition of `graph`.
    ///
    /// `tree_edges` are undirected pairs of indices into `bags`. For data
    /// whose validity is established by its construction, [`Self::new_trusted`]
    /// skips full validation in release builds.
    pub fn new(
        graph: &Graph,
        bags: impl IntoIterator<Item = Vec<u32>>,
        tree_edges: impl IntoIterator<Item = (usize, usize)>,
    ) -> Result<Self, Error> {
        let td = Self::assemble(graph.num_vertices, bags, tree_edges)?;
        td.validate(graph)?;
        Ok(td)
    }

    /// Build a decomposition supplied by a trusted algorithm.
    ///
    /// The caller must ensure that the bags and edges form a valid tree
    /// decomposition of `graph`. Debug builds check this contract; release
    /// builds skip full validation. Call [`Self::validate`] explicitly when
    /// a release build also needs the check, or use [`Self::new`] for data
    /// whose validity is not established.
    ///
    /// Bags and tree edges are canonicalized as in [`Self::new`]. Out-of-range
    /// bag-tree endpoints return an error in every build.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if the decomposition is invalid. Supplying
    /// invalid data in release builds may cause later operations to panic or
    /// produce incorrect results.
    pub fn new_trusted(
        graph: &Graph,
        bags: impl IntoIterator<Item = Vec<u32>>,
        tree_edges: impl IntoIterator<Item = (usize, usize)>,
    ) -> Result<Self, Error> {
        let td = Self::assemble(graph.num_vertices, bags, tree_edges)?;
        td.debug_validate(graph);
        Ok(td)
    }

    fn assemble(
        num_vertices: u32,
        bags: impl IntoIterator<Item = Vec<u32>>,
        tree_edges: impl IntoIterator<Item = (usize, usize)>,
    ) -> Result<Self, Error> {
        let bags: Vec<TdBag> = bags.into_iter().map(TdBag::new).collect();
        let mut tree_edges: Vec<(usize, usize)> = tree_edges
            .into_iter()
            .map(|(left, right)| (left.min(right), left.max(right)))
            .collect();
        tree_edges.sort_unstable();
        let mut adj = vec![Vec::new(); bags.len()];
        for (left, right) in tree_edges {
            if left >= bags.len() || right >= bags.len() {
                return invalid(format!(
                    "bag-tree edge ({left}, {right}) is outside 0..{}",
                    bags.len()
                ));
            }
            adj[left].push(right);
            adj[right].push(left);
        }
        Ok(Self::from_parts(num_vertices, bags, adj))
    }

    pub(crate) fn debug_validate(&self, graph: &Graph) {
        debug_assert!(
            self.validate(graph).is_ok(),
            "invalid trusted decomposition"
        );
    }

    pub(crate) fn from_parts(num_vertices: u32, bags: Vec<TdBag>, adj: Vec<Vec<usize>>) -> Self {
        Self {
            num_vertices,
            bags,
            adj,
        }
    }

    /// Number of vertices in the graph this decomposition was built for.
    pub fn num_vertices(&self) -> u32 {
        self.num_vertices
    }

    /// Bags in index order.
    pub fn bags(&self) -> &[TdBag] {
        &self.bags
    }

    /// Undirected bag adjacency, indexed like [`Self::bags`]. A decomposition
    /// built with [`Self::new`] has neighbours sorted by bag index; algorithms
    /// constructing a decomposition directly may expose their stable traversal
    /// order instead.
    pub fn adjacency(&self) -> &[Vec<usize>] {
        &self.adj
    }

    /// This decomposition's width: the vertices in its largest bag, less one.
    /// `0` where there is nothing to separate — no bags, one empty bag and one
    /// single-vertex bag alike.
    ///
    /// An upper bound on the decomposed graph's treewidth, which is the
    /// minimum width over all of its decompositions.
    pub fn treewidth(&self) -> u32 {
        self.bags
            .iter()
            .map(|b| b.vertices.len() as u32)
            .max()
            .unwrap_or(0)
            .saturating_sub(1)
    }

    /// Sum of bag sizes: the secondary quality signal beside the width. Two
    /// decompositions of equal width can have very different total bag volume.
    pub fn total_bag_size(&self) -> usize {
        self.bags.iter().map(|b| b.vertices.len()).sum()
    }

    /// Two more numbers about the decomposition's shape, for a caller that
    /// ranks candidates on something other than width.
    ///
    /// The mass is `log2` of the sum over bags of `2^|bag|`, which is what a
    /// consumer compiling over the bags pays in the worst case; the sum is
    /// scaled by the largest bag before the logarithm, so a bag of a few
    /// thousand vertices does not overflow the exponent. No bags is `0.0`.
    /// The separator is the largest number of vertices two adjacent bags
    /// share, which is what a consumer joining two bags carries between them.
    ///
    /// One pass over the bags and one over the tree edges, with a stamp array
    /// over the graph's vertices for the intersections.
    pub(crate) fn shape(&self) -> (f64, usize) {
        let Some(largest) = self.bags.iter().map(|bag| bag.vertices.len()).max() else {
            return (0.0, 0);
        };
        let scaled: f64 = self
            .bags
            .iter()
            .map(|bag| (bag.vertices.len() as f64 - largest as f64).exp2())
            .sum();
        let mass = largest as f64 + scaled.log2();

        let mut stamp = vec![usize::MAX; self.num_vertices as usize];
        let mut separator = 0;
        for (index, neighbours) in self.adj.iter().enumerate() {
            for &vertex in &self.bags[index].vertices {
                stamp[vertex as usize] = index;
            }
            // Each edge is walked from both ends; the shared count is the same
            // either way, so taking the maximum over all of them is enough.
            for &neighbour in neighbours {
                let shared = self.bags[neighbour]
                    .vertices
                    .iter()
                    .filter(|&&vertex| stamp[vertex as usize] == index)
                    .count();
                separator = separator.max(shared);
            }
        }
        (mass, separator)
    }

    /// The ordering used when goatd compares two decompositions: narrower
    /// first, then fewer total vertices across all bags.
    ///
    /// The same numbers as [`Self::treewidth`] and [`Self::total_bag_size`],
    /// read in one pass over the bags because the portfolio asks for both of
    /// them on every candidate it produces.
    pub(crate) fn quality_key(&self) -> (u32, usize) {
        let mut largest = 0usize;
        let mut total = 0usize;
        for bag in &self.bags {
            let size = bag.vertices.len();
            largest = largest.max(size);
            total += size;
        }
        ((largest as u32).saturating_sub(1), total)
    }

    /// Check that this is a tree decomposition of `graph`.
    ///
    /// This checks bag contents and acyclic bag adjacency, vertex and edge
    /// coverage, and the running intersection property. An empty decomposition
    /// is valid for an empty graph.
    pub fn validate(&self, graph: &Graph) -> Result<(), Error> {
        if self.num_vertices != graph.num_vertices {
            return invalid(format!(
                "the decomposition is for {} vertices but the graph has {}",
                self.num_vertices, graph.num_vertices
            ));
        }
        let num_bags = self.bags.len();
        if self.adj.len() != num_bags {
            return invalid(format!(
                "the decomposition has {num_bags} bags but {} adjacency lists",
                self.adj.len()
            ));
        }

        if num_bags == 0 {
            return if graph.num_vertices == 0 {
                Ok(())
            } else {
                invalid("vertex 0 is in no bag")
            };
        }

        // Which bags hold each vertex, as one flat array rather than a `Vec`
        // per vertex. The first pass checks each bag's contents and counts the
        // holders, the second places them through prefix-sum offsets. Bags are
        // visited in ascending order in both. `seen_in_bag`
        // carries the position of the bag a vertex was last seen in, which
        // catches a repeat within one bag without a set per bag.
        let num_vertices = graph.num_vertices as usize;
        let mut holder_offsets = vec![0usize; num_vertices + 1];
        let mut seen_in_bag = vec![usize::MAX; num_vertices];
        for (position, bag) in self.bags.iter().enumerate() {
            for &vertex in &bag.vertices {
                if vertex >= graph.num_vertices {
                    return invalid(format!(
                        "bag {position} contains vertex {vertex}, outside 0..{}",
                        graph.num_vertices
                    ));
                }
                if seen_in_bag[vertex as usize] == position {
                    return invalid(format!(
                        "bag {position} contains vertex {vertex} more than once"
                    ));
                }
                seen_in_bag[vertex as usize] = position;
                holder_offsets[vertex as usize + 1] += 1;
            }
        }
        for vertex in 0..num_vertices {
            holder_offsets[vertex + 1] += holder_offsets[vertex];
        }
        let mut holder_bags = vec![0usize; holder_offsets[num_vertices]];
        let mut holder_cursor = holder_offsets[..num_vertices].to_vec();
        for (position, bag) in self.bags.iter().enumerate() {
            for &vertex in &bag.vertices {
                holder_bags[holder_cursor[vertex as usize]] = position;
                holder_cursor[vertex as usize] += 1;
            }
        }

        let mut arcs = FxHashSet::default();
        for (bag, neighbours) in self.adj.iter().enumerate() {
            for &neighbour in neighbours {
                if neighbour >= num_bags {
                    return invalid(format!(
                        "bag {bag} has neighbour {neighbour}, but there are {num_bags} bags"
                    ));
                }
                if neighbour == bag {
                    return invalid(format!("bag {bag} is adjacent to itself"));
                }
                if !arcs.insert((bag, neighbour)) {
                    return invalid(format!(
                        "bag {neighbour} occurs more than once in adjacency list {bag}"
                    ));
                }
            }
        }
        for &(bag, neighbour) in &arcs {
            if !arcs.contains(&(neighbour, bag)) {
                return invalid(format!(
                    "bag {bag} names {neighbour} as a neighbour, but the reverse edge is missing"
                ));
            }
        }

        let mut seen = vec![false; num_bags];
        let mut parent = vec![usize::MAX; num_bags];
        let mut depth = vec![0usize; num_bags];
        let mut num_components = 0usize;
        for start in 0..num_bags {
            if seen[start] {
                continue;
            }
            num_components += 1;
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(bag) = stack.pop() {
                for &neighbour in &self.adj[bag] {
                    if !seen[neighbour] {
                        seen[neighbour] = true;
                        parent[neighbour] = bag;
                        depth[neighbour] = depth[bag] + 1;
                        stack.push(neighbour);
                    }
                }
            }
        }
        let num_tree_edges = arcs.len() / 2;
        let forest_edges = num_bags - num_components;
        if num_tree_edges != forest_edges {
            return invalid(format!(
                "the bag graph has {num_tree_edges} edges; a forest of {num_components} components on {num_bags} bags has {forest_edges}"
            ));
        }

        for vertex in 0..num_vertices {
            if holder_run(&holder_offsets, &holder_bags, vertex).is_empty() {
                return invalid(format!("vertex {vertex} is in no bag"));
            }
        }

        // In a rooted forest, a nonempty set of bags is connected exactly
        // when only one of its bags has no parent in the set. That bag is
        // the root-most holder. Marking holders avoids walking the neighbours
        // of a high-degree bag once for every vertex it contains.
        let mut top = vec![usize::MAX; num_vertices];
        let mut holding_mark = vec![usize::MAX; num_bags];
        for (vertex, vertex_top) in top.iter_mut().enumerate() {
            let vertex_holders = holder_run(&holder_offsets, &holder_bags, vertex);
            for &bag in vertex_holders {
                holding_mark[bag] = vertex;
            }
            for &bag in vertex_holders {
                if parent[bag] == usize::MAX || holding_mark[parent[bag]] != vertex {
                    if *vertex_top != usize::MAX {
                        return invalid(format!(
                            "the bags holding vertex {vertex} are not connected"
                        ));
                    }
                    *vertex_top = bag;
                }
            }
        }

        // Two connected holder subtrees intersect iff the deeper root-most
        // holder contains both vertices. Group edges by that bag, then mark
        // its vertices once to check every assigned edge in constant time.
        let mut first_edge = vec![usize::MAX; num_bags];
        let mut next_edge = vec![usize::MAX; graph.edges.len()];
        for (edge, &(u, v)) in graph.edges.iter().enumerate() {
            if u >= graph.num_vertices || v >= graph.num_vertices {
                return invalid(format!(
                    "graph edge ({u}, {v}) has an endpoint outside 0..{}",
                    graph.num_vertices
                ));
            }
            let (left, right) = (top[u as usize], top[v as usize]);
            let bag = if depth[left] >= depth[right] {
                left
            } else {
                right
            };
            next_edge[edge] = first_edge[bag];
            first_edge[bag] = edge;
        }
        seen_in_bag.fill(usize::MAX);
        for (position, bag) in self.bags.iter().enumerate() {
            for &vertex in &bag.vertices {
                seen_in_bag[vertex as usize] = position;
            }
            let mut edge = first_edge[position];
            while edge != usize::MAX {
                let (u, v) = graph.edges[edge];
                if seen_in_bag[u as usize] != position || seen_in_bag[v as usize] != position {
                    return invalid(format!("edge ({u}, {v}) is covered by no bag"));
                }
                edge = next_edge[edge];
            }
        }

        Ok(())
    }
}

/// The bags holding `vertex`, as a run of the flat holder array `validate`
/// builds.
fn holder_run<'a>(offsets: &[usize], bags: &'a [usize], vertex: usize) -> &'a [usize] {
    &bags[offsets[vertex]..offsets[vertex + 1]]
}

fn invalid<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error::InvalidDecomposition(message.into()))
}

#[cfg(test)]
mod tests;
