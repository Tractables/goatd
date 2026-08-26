//! The tree decomposition type.

use rustc_hash::FxHashSet;

use crate::{Error, Graph};

/// One bag of a tree decomposition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TdBag {
    /// Index of this bag within [`TreeDecomposition::bags`]; also the index used
    /// by [`TreeDecomposition::adj`].
    pub id: usize,
    /// The bag's vertices, 0-indexed (PACE `.td` vertex ids minus one).
    pub vertices: Vec<u32>,
}

/// A tree decomposition: bags of vertices, and an acyclic adjacency over them.
///
/// Every function in this crate that returns one preserves the running
/// intersection property — the bags holding any one vertex form a connected
/// subtree — and covers every vertex and every edge of the graph it decomposed.
/// A disconnected graph may have one bag-tree component per graph component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeDecomposition {
    /// The bags, indexed by [`TdBag::id`].
    pub bags: Vec<TdBag>,
    /// Acyclic adjacency between bags, indexed like `bags`: `adj[i]` lists the
    /// bag indices connected to bag `i`.
    pub adj: Vec<Vec<usize>>,
}

impl TreeDecomposition {
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

    /// Check that this is a tree decomposition of `graph`.
    ///
    /// This checks the bag ids and acyclic bag adjacency, vertex and edge coverage,
    /// and the running intersection property. An empty decomposition is valid
    /// for an empty graph.
    pub fn validate(&self, graph: &Graph) -> Result<(), Error> {
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
            if bag.id != position {
                return invalid(format!("bag {} is stored at position {position}", bag.id));
            }
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
            if !holders[u as usize]
                .iter()
                .any(|&bag| self.bags[bag].vertices.contains(&v))
            {
                return invalid(format!("edge ({u}, {v}) is covered by no bag"));
            }
        }

        for (vertex, vertex_holders) in holders.iter().enumerate() {
            if vertex_holders.len() < 2 {
                continue;
            }
            let holding: FxHashSet<usize> = vertex_holders.iter().copied().collect();
            let mut reached = FxHashSet::default();
            let mut stack = vec![vertex_holders[0]];
            reached.insert(vertex_holders[0]);
            while let Some(bag) = stack.pop() {
                for &neighbour in &self.adj[bag] {
                    if holding.contains(&neighbour) && reached.insert(neighbour) {
                        stack.push(neighbour);
                    }
                }
            }
            if reached.len() != vertex_holders.len() {
                return invalid(format!(
                    "the bags holding vertex {vertex} are not connected"
                ));
            }
        }

        Ok(())
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error::InvalidDecomposition(message.into()))
}
