//! Multilevel two-way partitioning for graphs and hypergraphs.
//!
//! [`multilevel_graph_bisect`] minimizes an edge cut on a simple graph;
//! [`multilevel_hypergraph_bisect`] minimizes a cut in which each split
//! hyperedge is counted once. Both return a [`Bisection`] and use deterministic
//! seeded search.

mod common;
mod graph;
mod hypergraph;

pub use graph::{GraphBisectionConfig, multilevel_graph_bisect};
pub use hypergraph::{Hypergraph, HypergraphBisectionConfig, multilevel_hypergraph_bisect};

/// A two-way vertex partition.
///
/// `parts()[vertex]` is `0` or `1`. Both values occur when the input has at
/// least two vertices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bisection {
    parts: Vec<u8>,
}

impl Bisection {
    pub(super) fn new(parts: Vec<u8>) -> Self {
        Self { parts }
    }

    /// One side number, `0` or `1`, per input vertex.
    pub fn parts(&self) -> &[u8] {
        &self.parts
    }

    /// Consume the bisection and return its side numbers.
    pub fn into_parts(self) -> Vec<u8> {
        self.parts
    }
}
