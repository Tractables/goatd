//! Decompose the projection onto one side of a bipartite graph, then put the
//! other side back.
//!
//! On a bipartite graph one side `S` is an independent set, so eliminating any
//! subset `D` of `S` first adds no edge inside `D` and each of its vertices
//! leaves a bag of its own neighbourhood plus itself. What is left is the
//! projection: everything outside `D`, with every neighbourhood of a `D`
//! vertex turned into a clique. Decomposing the projection and putting `D`
//! back therefore gives a decomposition of the whole graph of width
//!
//! ```text
//! max(width of the projection, largest degree over D)
//! ```
//!
//! Eliminating the whole side is the cheapest projection to search and the
//! weakest bound: a single vertex of degree d makes the projection hold a
//! clique of size d, so on an incidence graph the projection is the primal
//! graph and the result can never be narrower than the primal width, however
//! much narrower the incidence graph is. Keeping the vertices of degree above
//! a cutoff, so that only the short ones are eliminated, gives a larger graph
//! to search and a bound between the two. So the stage does not choose one
//! cutoff: it walks the quantiles of the side's own degrees from the largest
//! down, spending on each what its share of the work is worth, and keeps the
//! narrowest lift any of them produced. The first rung is the whole side.
//!
//! Every rung is measured before anything is built: eliminating vertices of
//! degrees d costs the sum of d(d-1)/2, which bounds the projection's clique
//! edges and is the work of building them, plus the edges of the vertices the
//! cutoff keeps. Only the cheaper side is built, and the other only if that
//! one turns out to hold more edges than the input. A side whose projection
//! would keep too large a share of the vertices is not measured at all:
//! decomposing nearly the same graph again is what the stage is spending its
//! share of the window on.
//!
//! Putting a vertex back needs a bag holding all of its neighbours, and one
//! exists because those neighbours are a clique of the projection. The bag is
//! found through the rooted bag tree: of the neighbours, take the one whose
//! shallowest bag is deepest, and that bag holds the rest of them too. This
//! module checks that rather than trusting it, so a defect here surfaces as an
//! error instead of an invalid decomposition.

use crate::{Error, Graph, TreeDecomposition};

/// The input's adjacency lists, which both the colouring and the projection
/// read several times.
pub(super) fn adjacency(graph: &Graph) -> Vec<Vec<u32>> {
    crate::meter::charge(graph.edges().len() as u64);
    let mut adjacency = vec![Vec::new(); graph.num_vertices() as usize];
    for &(left, right) in graph.edges() {
        adjacency[left as usize].push(right);
        adjacency[right as usize].push(left);
    }
    adjacency
}

/// The two sides of a 2-colouring, or `None` when the graph has an odd cycle.
/// Each component is coloured on its own, which is all the lift needs: either
/// side is an independent set whatever the components did.
pub(super) fn sides(adjacency: &[Vec<u32>]) -> Option<[Vec<u32>; 2]> {
    crate::meter::charge(adjacency.len() as u64);
    let mut colour = vec![u8::MAX; adjacency.len()];
    let mut queue = Vec::new();
    let mut out = [Vec::new(), Vec::new()];
    for start in 0..adjacency.len() {
        if colour[start] != u8::MAX {
            continue;
        }
        colour[start] = 0;
        out[0].push(start as u32);
        queue.push(start as u32);
        while let Some(vertex) = queue.pop() {
            let next = 1 - colour[vertex as usize];
            for &neighbour in &adjacency[vertex as usize] {
                if colour[neighbour as usize] == u8::MAX {
                    colour[neighbour as usize] = next;
                    out[next as usize].push(neighbour);
                    queue.push(neighbour);
                } else if colour[neighbour as usize] == colour[vertex as usize] {
                    return None;
                }
            }
        }
    }
    Some(out)
}

/// The graph left when part of one side is eliminated, and what it takes to
/// put those vertices back.
pub(super) struct Projection {
    /// Projection vertex `i` is input vertex `vertices[i]`.
    vertices: Vec<u32>,
    /// The projection itself: everything that was not eliminated, with each
    /// eliminated vertex's neighbourhood a clique.
    pub(super) graph: Graph,
    /// One entry per eliminated vertex: its input id and its neighbourhood in
    /// projection ids.
    eliminated: Vec<(u32, Vec<u32>)>,
    /// The largest neighbourhood over the eliminated side, which is the width
    /// those vertices contribute.
    pub(super) eliminated_width: u32,
}

/// What one rung of the ladder costs: eliminating the vertices of a side whose
/// degree is at most `cutoff`, and keeping the rest.
#[derive(Clone, Copy, Debug)]
pub(super) struct Price {
    /// Vertices of the side with a degree above this are kept rather than
    /// eliminated. [`u32::MAX`] eliminates the whole side.
    pub(super) cutoff: u32,
    /// The clique edges the eliminations add, counted with their repeats.
    pub(super) pairs: usize,
    /// The input edges the kept vertices of the side bring with them.
    pub(super) kept_edges: usize,
    /// How many vertices of the side the cutoff keeps.
    pub(super) kept: usize,
}

impl Price {
    /// Edges of work: what building this projection costs, and what stands for
    /// the size of the search over it.
    pub(super) fn work(self) -> usize {
        self.pairs.saturating_add(self.kept_edges)
    }
}

/// What eliminating the short vertices of `side` would cost, or `None` when
/// the clique edges are over `limit`.
///
/// The clique of an eliminated vertex of degree d holds d(d-1)/2 edges, and
/// they are counted with their repeats: a pair two eliminations both cover is
/// one edge of the projection, but it costs the work of two here. So `pairs`
/// is an upper bound on the clique edges of the projection and the exact cost
/// of building them, and it is what decides which side to build.
pub(super) fn price(
    adjacency: &[Vec<u32>],
    side: &[u32],
    cutoff: u32,
    limit: usize,
) -> Option<Price> {
    let mut priced = Price {
        cutoff,
        pairs: 0,
        kept_edges: 0,
        kept: 0,
    };
    for &vertex in side {
        let degree = adjacency[vertex as usize].len();
        if degree as u64 > u64::from(cutoff) {
            priced.kept += 1;
            priced.kept_edges = priced.kept_edges.saturating_add(degree);
            continue;
        }
        priced.pairs = priced
            .pairs
            .saturating_add(degree * degree.saturating_sub(1) / 2);
        if priced.pairs > limit {
            return None;
        }
    }
    Some(priced)
}

/// The quantiles of a side's degrees the cutoffs are taken from, after the
/// whole side.
///
/// They stop at the top tenth. A cutoff further down keeps most of the side,
/// which is the input graph again, and the rest of the schedule already
/// searches that with the whole window instead of a rung's slice of a share.
const CUTOFF_QUANTILES: [f64; 2] = [0.99, 0.90];

/// The cutoffs to walk on `side`, largest first.
///
/// The rungs are quantiles of the side's own degrees, so a side whose degrees
/// are all the same has one rung and the stage does exactly what eliminating
/// the whole side does. The first rung is always the whole side, which is the
/// smallest projection to search and the weakest bound; each further rung
/// keeps the vertices above its cutoff, which is a larger graph to search and
/// a bound no higher.
pub(super) fn cutoffs(adjacency: &[Vec<u32>], side: &[u32]) -> Vec<u32> {
    crate::meter::charge(side.len() as u64);
    let mut degrees: Vec<u32> = side
        .iter()
        .map(|&vertex| adjacency[vertex as usize].len() as u32)
        .collect();
    degrees.sort_unstable();
    let widest = degrees[degrees.len() - 1];
    let mut out = vec![u32::MAX];
    for quantile in CUTOFF_QUANTILES {
        let index = (quantile * (degrees.len() - 1) as f64) as usize;
        let cutoff = degrees[index];
        // A rung that eliminates nothing leaves the input graph, one that
        // eliminates everything repeats the whole side, and one that repeats
        // the rung above it is the same search twice.
        if cutoff > 0 && cutoff < widest && !out.contains(&cutoff) {
            out.push(cutoff);
        }
    }
    out
}

/// Project `graph`, keeping `keep` and the vertices of `drop` that `priced`
/// keeps, or `None` when the projection holds more edges than the input.
///
/// `keep` and `drop` are the two sides of a 2-colouring, so every edge of the
/// input runs between them: the projection's edges are the cliques the
/// eliminations leave behind, plus the input's own edges at the vertices of
/// `drop` the cutoff kept. `priced` is what [`price`] returned for this side
/// and cutoff.
///
/// A projection is worth decomposing only if it is smaller than the input:
/// fewer vertices, which eliminating something always gives, and no more
/// edges, which it often does not. On the incidence graph of a formula the
/// whole clause side projects onto the primal graph, which is smaller on both
/// counts; on a grid the same construction turns every degree-4 vertex into
/// six edges and leaves more edges than it started with, and there the lift is
/// work for nothing.
pub(super) fn project(
    graph: &Graph,
    adjacency: &[Vec<u32>],
    keep: &[u32],
    drop: &[u32],
    priced: Price,
) -> Option<Projection> {
    crate::meter::charge(priced.work() as u64);

    let mut local = vec![u32::MAX; graph.num_vertices() as usize];
    let mut vertices = Vec::with_capacity(keep.len() + priced.kept);
    for &vertex in keep.iter().chain(
        drop.iter()
            .filter(|&&vertex| adjacency[vertex as usize].len() as u64 > u64::from(priced.cutoff)),
    ) {
        local[vertex as usize] = vertices.len() as u32;
        vertices.push(vertex);
    }
    let mut edges = Vec::with_capacity(priced.work());
    let mut eliminated = Vec::with_capacity(drop.len() - priced.kept);
    let mut eliminated_width = 0u32;
    for &vertex in drop {
        if local[vertex as usize] != u32::MAX {
            // A vertex the cutoff kept: its own edges stay as they are.
            let here = local[vertex as usize];
            edges.extend(
                adjacency[vertex as usize]
                    .iter()
                    .map(|&neighbour| (here, local[neighbour as usize])),
            );
            continue;
        }
        let neighbourhood: Vec<u32> = adjacency[vertex as usize]
            .iter()
            .map(|&neighbour| local[neighbour as usize])
            .collect();
        debug_assert!(neighbourhood.iter().all(|&n| n != u32::MAX));
        eliminated_width = eliminated_width.max(neighbourhood.len() as u32);
        for (offset, &left) in neighbourhood.iter().enumerate() {
            for &right in &neighbourhood[offset + 1..] {
                edges.push((left, right));
            }
        }
        eliminated.push((vertex, neighbourhood));
    }
    let projected = Graph::new(vertices.len() as u32, edges);
    if projected.edges().len() > graph.edges().len() {
        return None;
    }
    Some(Projection {
        graph: projected,
        vertices,
        eliminated,
        eliminated_width,
    })
}

impl Projection {
    /// Weights for the projection's vertices, taken from the input's.
    pub(super) fn weights(&self, weights: &[u32]) -> Vec<u32> {
        self.vertices
            .iter()
            .map(|&vertex| weights[vertex as usize])
            .collect()
    }

    /// The width the lift of `projection` would have, without building it.
    pub(super) fn lifted_width(&self, projection: &TreeDecomposition) -> u32 {
        projection.treewidth().max(self.eliminated_width)
    }

    /// Put the eliminated side back into a decomposition of the projection.
    ///
    /// # Errors
    ///
    /// Returns an error when no bag holds an eliminated vertex's neighbourhood,
    /// which cannot happen for a decomposition of this projection, or when the
    /// result does not validate against `graph`.
    pub(super) fn lift(
        &self,
        graph: &Graph,
        projection: &TreeDecomposition,
    ) -> Result<TreeDecomposition, Error> {
        crate::meter::charge(projection.total_bag_size() as u64);
        let mut bags: Vec<Vec<u32>> = projection
            .bags()
            .iter()
            .map(|bag| {
                bag.vertices()
                    .iter()
                    .map(|&vertex| self.vertices[vertex as usize])
                    .collect()
            })
            .collect();
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for (bag, neighbours) in projection.adjacency().iter().enumerate() {
            for &neighbour in neighbours {
                if bag < neighbour {
                    edges.push((bag, neighbour));
                }
            }
        }

        let (depth, shallowest) = self.rooted_depths(projection);
        // One stamp per projection vertex, so the containment check below
        // costs the size of the neighbourhood rather than the size of the bag.
        let mut stamp = vec![u32::MAX; self.vertices.len()];
        for (index, &(vertex, ref neighbourhood)) in self.eliminated.iter().enumerate() {
            let index = index as u32;
            if neighbourhood.is_empty() {
                bags.push(vec![vertex]);
                continue;
            }
            let host = neighbourhood
                .iter()
                .map(|&neighbour| shallowest[neighbour as usize])
                .max_by_key(|&bag| depth[bag])
                .expect("a non-empty neighbourhood has a deepest bag");
            for &member in projection.bags()[host].vertices() {
                stamp[member as usize] = index;
            }
            if let Some(&missing) = neighbourhood
                .iter()
                .find(|&&member| stamp[member as usize] != index)
            {
                return Err(Error::InvalidInput(format!(
                    "the bipartite lift found no bag holding the neighbourhood of vertex \
                     {vertex}: bag {host} is missing {}",
                    self.vertices[missing as usize]
                )));
            }
            let mut bag = Vec::with_capacity(neighbourhood.len() + 1);
            bag.push(vertex);
            bag.extend(
                neighbourhood
                    .iter()
                    .map(|&neighbour| self.vertices[neighbour as usize]),
            );
            bags.push(bag);
            edges.push((bags.len() - 1, host));
        }
        TreeDecomposition::new(graph, bags, edges)
    }

    /// Depth of every bag from the root of its component, and for every
    /// projection vertex the shallowest bag holding it.
    fn rooted_depths(&self, projection: &TreeDecomposition) -> (Vec<u32>, Vec<usize>) {
        let num_bags = projection.bags().len();
        let mut depth = vec![u32::MAX; num_bags];
        let mut shallowest = vec![usize::MAX; self.vertices.len()];
        let mut queue = std::collections::VecDeque::new();
        for start in 0..num_bags {
            if depth[start] != u32::MAX {
                continue;
            }
            depth[start] = 0;
            queue.push_back(start);
            while let Some(bag) = queue.pop_front() {
                for &vertex in projection.bags()[bag].vertices() {
                    if shallowest[vertex as usize] == usize::MAX {
                        shallowest[vertex as usize] = bag;
                    }
                }
                for &neighbour in &projection.adjacency()[bag] {
                    if depth[neighbour] == u32::MAX {
                        depth[neighbour] = depth[bag] + 1;
                        queue.push_back(neighbour);
                    }
                }
            }
        }
        (depth, shallowest)
    }
}

#[cfg(test)]
mod tests;
