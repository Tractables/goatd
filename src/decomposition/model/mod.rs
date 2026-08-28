//! Tree-decomposition data and validation.

use rustc_hash::FxHashSet;

use crate::{Error, Graph};

/// One bag of a tree decomposition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TdBag {
    /// The bag's vertices, 0-indexed (PACE `.td` vertex ids minus one).
    pub(crate) vertices: Vec<u32>,
}

impl TdBag {
    pub(crate) fn new(mut vertices: Vec<u32>) -> Self {
        vertices.sort_unstable();
        Self { vertices }
    }

    /// Vertices in this bag, sorted in ascending order.
    pub fn vertices(&self) -> &[u32] {
        &self.vertices
    }
}

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
    /// `tree_edges` are undirected pairs of indices into `bags`.
    pub fn new(
        graph: &Graph,
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
        let td = Self::from_parts(graph.num_vertices, bags, adj);
        td.validate(graph)?;
        Ok(td)
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

    /// The ordering used when goatd compares two decompositions: narrower
    /// first, then fewer total vertices across all bags.
    pub(crate) fn quality_key(&self) -> (u32, usize) {
        (self.treewidth(), self.total_bag_size())
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

        let mut holders = vec![Vec::new(); graph.num_vertices as usize];
        for (position, bag) in self.bags.iter().enumerate() {
            let mut in_bag = FxHashSet::default();
            for &vertex in &bag.vertices {
                if vertex >= graph.num_vertices {
                    return invalid(format!(
                        "bag {position} contains vertex {vertex}, outside 0..{}",
                        graph.num_vertices
                    ));
                }
                if !in_bag.insert(vertex) {
                    return invalid(format!(
                        "bag {position} contains vertex {vertex} more than once"
                    ));
                }
                holders[vertex as usize].push(position);
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

        for (vertex, vertex_holders) in holders.iter().enumerate() {
            if vertex_holders.is_empty() {
                return invalid(format!("vertex {vertex} is in no bag"));
            }
        }

        for &(u, v) in &graph.edges {
            if u >= graph.num_vertices || v >= graph.num_vertices {
                return invalid(format!(
                    "graph edge ({u}, {v}) has an endpoint outside 0..{}",
                    graph.num_vertices
                ));
            }
            if !sorted_lists_intersect(&holders[u as usize], &holders[v as usize]) {
                return invalid(format!("edge ({u}, {v}) is covered by no bag"));
            }
        }

        let mut holding_mark = vec![0usize; num_bags];
        let mut reached_mark = vec![0usize; num_bags];
        for (vertex, vertex_holders) in holders.iter().enumerate() {
            if vertex_holders.len() < 2 {
                continue;
            }
            let mark = vertex + 1;
            for &bag in vertex_holders {
                holding_mark[bag] = mark;
            }
            let mut stack = vec![vertex_holders[0]];
            reached_mark[vertex_holders[0]] = mark;
            let mut reached = 1usize;
            while let Some(bag) = stack.pop() {
                for &neighbour in &self.adj[bag] {
                    if holding_mark[neighbour] == mark && reached_mark[neighbour] != mark {
                        reached_mark[neighbour] = mark;
                        reached += 1;
                        stack.push(neighbour);
                    }
                }
            }
            if reached != vertex_holders.len() {
                return invalid(format!(
                    "the bags holding vertex {vertex} are not connected"
                ));
            }
        }

        Ok(())
    }
}

fn sorted_lists_intersect(left: &[usize], right: &[usize]) -> bool {
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
            std::cmp::Ordering::Equal => return true,
        }
    }
    false
}

fn invalid<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error::InvalidDecomposition(message.into()))
}

#[cfg(test)]
mod tests;
