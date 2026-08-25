//! The tree decomposition type.

/// One bag of a tree decomposition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TdBag {
    /// Index of this bag within [`TreeDecomposition::bags`]; also the index used
    /// by [`TreeDecomposition::adj`].
    pub id: usize,
    /// The bag's vertices, 0-indexed (PACE `.td` vertex ids minus one).
    pub vertices: Vec<u32>,
}

/// A tree decomposition: bags of vertices, and the tree over them.
///
/// Every function in this crate that returns one preserves the running
/// intersection property — the bags holding any one vertex form a connected
/// subtree — and covers every vertex and every edge of the graph it decomposed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeDecomposition {
    /// The bags, indexed by [`TdBag::id`].
    pub bags: Vec<TdBag>,
    /// Tree adjacency between bags, indexed like `bags`: `adj[i]` lists the
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
}
