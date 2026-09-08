//! Build a decomposition out of bags taken from several others.
//!
//! Every construction here produces a whole tree decomposition and the caller
//! keeps the narrowest. That throws away the good bags of every other one. This
//! module keeps them instead: it holds the best few decompositions a run
//! produced, pools their bags, and searches that pool for the narrowest tree
//! decomposition whose every bag comes from it.
//!
//! The pool gives each decomposition it keeps an equal share of its bags, so no
//! one of them fills it, and it takes nothing from a decomposition that does
//! not fit its share. Each is minimalised on the way in where there is time for
//! it, so the bags pooled are the cliques of a minimal triangulation rather
//! than whatever the elimination left.
//!
//! The search is the dynamic programme of Bouchitté and Todinca, "Treewidth and
//! minimum fill-in: grouping the minimal separators", SIAM Journal on Computing
//! 31(1), 2001, restricted to a list of candidate bags rather than run over all
//! the potential maximal cliques of the graph. Tamaki, "Computing treewidth via
//! exact and heuristic lists of minimal separators", 2019, is where the
//! restricted form comes from.
//!
//! The two objects it works with:
//!
//! - a **block** is a connected component `C` of `G − Ω` for a pool bag `Ω`,
//!   carried with its separator `N(C)`;
//! - a **cap** of a block `(C, S)` is a pool bag `Ω` with `S ⊆ Ω ⊆ C ∪ S` and
//!   at least one vertex inside `C`.
//!
//! The width of a block is the cheapest way to decompose `C ∪ S` with `S` in
//! its top bag:
//!
//! ```text
//! width(C, S) = min( |C| + |S| − 1,
//!                    min over caps Ω of
//!                      max( |Ω| − 1, width of each block of G − Ω inside C ) )
//! ```
//!
//! and the answer is the same expression over the whole graph, minimised over
//! the choice of top bag. Blocks are evaluated smallest first, so a block's
//! sub-blocks — strictly smaller — are settled before it and one pass is
//! enough.
//!
//! The tree that comes out is a valid decomposition whatever the pool holds. A
//! cap and the blocks below it cover every edge inside `C ∪ S`: an edge with
//! both ends in the cap is in that bag, an edge from the cap into a sub-block
//! has its cap end in that sub-block's separator, and two sub-blocks share no
//! edge. Each vertex's bags form a subtree because a block's bags stay inside
//! `C ∪ S`. So no bag has to be tested for being a potential maximal clique.
//! The pool always holds the bags of the run's own best decomposition, so the
//! search does not come back wider than that.
//!
//! After the first answer the widest pieces of it — a bag with the bags next to
//! it in the tree — are decomposed on their own by MCS-M and their cliques
//! added to the pool, and the programme runs again over the longer list. That
//! is the growth step of Tamaki's heuristic, and it stops at the deadline or
//! when a round adds nothing.
//!
//! What it costs: a traversal of the graph per pool bag, then one pass over the
//! blocks, per round. What it holds: the pool and one vertex list per block,
//! both capped by a constant (see [`Limits`]). It stops collecting rather than
//! exceed either cap and searches the part of the pool it took in, and it hands
//! back nothing at its deadline rather than a part-built answer.

use std::time::Instant;

use rustc_hash::{FxHashMap, FxHashSet};

use super::TreeDecomposition;
use crate::Graph;
use crate::deadline::expired;

#[cfg(test)]
mod tests;

/// Distinct bags the pool may hold.
const MAX_POOL_BAGS: usize = 4_000;
/// Vertex ids the pool may hold.
const MAX_POOL_VERTICES: usize = 1_000_000;
/// Vertex ids the block map may hold.
const MAX_BLOCK_VERTICES: usize = 32_000_000;
/// Decompositions the pool keeps, each with an equal share of the bags.
const POOL_CANDIDATES: usize = 4;
/// Rounds of growth after the first answer.
const GROWTH_ROUNDS: usize = 2;
/// Pieces re-decomposed per round.
const GROWTH_PIECES: usize = 8;
/// Blocks evaluated between two reads of the clock.
const DEADLINE_STRIDE: usize = 64;

/// What the pool and the search may hold.
///
/// The caps are constants rather than a function of the graph, so the stage's
/// memory does not grow with it: 4 MiB of pool and 128 MiB of blocks, doubled
/// by the map the blocks are looked up in. A graph whose pool would need more
/// is searched as far as the caps reach; what is settled by then still gives a
/// valid decomposition, only a wider one. The bag count is capped as well,
/// because the search costs one pass over the graph per bag, and it is shared
/// out: each of the [`Limits::candidates`] decompositions the pool keeps gets
/// the same quota, so no one of them fills the pool on its own.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pool_vertices: usize,
    block_vertices: usize,
    bags: usize,
    candidates: usize,
}

impl Limits {
    /// The caps every graph is searched under.
    pub(crate) fn standard() -> Self {
        Self {
            pool_vertices: MAX_POOL_VERTICES,
            block_vertices: MAX_BLOCK_VERTICES,
            bags: MAX_POOL_BAGS,
            candidates: POOL_CANDIDATES,
        }
    }

    /// How many decompositions the pool keeps.
    pub(crate) fn candidates(&self) -> usize {
        self.candidates
    }

    /// The bags one kept decomposition may contribute.
    pub(crate) fn quota(&self) -> usize {
        self.bags / self.candidates.max(1)
    }
}

/// The best few decompositions a run produced, kept whole so their bags can be
/// pooled together.
///
/// A decomposition is kept only if it fits its share of the pool, so a graph
/// whose candidates carry tens of thousands of bags keeps nothing and the
/// stage has nothing to do. What is kept is ranked the way the candidate set
/// ranks it, so the bags that go into the pool are the bags of the narrowest
/// decompositions the run has, not of the first ones it produced.
pub(crate) struct BagPool {
    kept: Vec<TreeDecomposition>,
    limits: Limits,
}

impl BagPool {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            kept: Vec::new(),
            limits,
        }
    }

    /// Offer one decomposition to the pool.
    pub(crate) fn absorb(&mut self, decomposition: &TreeDecomposition) {
        if decomposition.bags().len() > self.limits.quota() {
            return;
        }
        let key = decomposition.quality_key();
        let position = self.kept.partition_point(|kept| kept.quality_key() <= key);
        if position >= self.limits.candidates {
            return;
        }
        self.kept.insert(position, decomposition.clone());
        self.kept.truncate(self.limits.candidates);
    }

    /// The bags of everything kept, narrowest decomposition first, each one
    /// minimalised where there is time for it so its bags are the cliques of a
    /// minimal triangulation rather than whatever the elimination left.
    fn assemble(&self, graph: &Graph, deadline: Option<Instant>) -> Vec<Vec<u32>> {
        let mut bags: Vec<Vec<u32>> = Vec::new();
        let mut seen: FxHashSet<Vec<u32>> = FxHashSet::default();
        let mut stored = 0usize;
        for kept in &self.kept {
            if expired(deadline) {
                break;
            }
            let source = if super::minimalize_fits(kept, graph, deadline) {
                super::minimalize_at(kept.clone(), graph, deadline)
            } else {
                kept.clone()
            };
            for bag in source.bags() {
                if bags.len() >= self.limits.bags || stored >= self.limits.pool_vertices {
                    return bags;
                }
                let mut vertices = bag.vertices().to_vec();
                vertices.sort_unstable();
                vertices.dedup();
                if seen.contains(&vertices) {
                    continue;
                }
                stored += vertices.len();
                seen.insert(vertices.clone());
                bags.push(vertices);
            }
        }
        bags
    }

    /// How many decompositions the pool holds.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.kept.len()
    }

    /// Whether the pool holds nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }
}

/// One block: a connected component of `G − Ω` for some pool bag, with its
/// separator, the pool bags that cap it, and what the dynamic programme
/// settled.
struct Block {
    component: Vec<u32>,
    separator: Vec<u32>,
    caps: Vec<usize>,
    width: u32,
    /// The cap the width came from, or `None` where `C ∪ S` in one bag was
    /// cheapest.
    best_cap: Option<usize>,
}

/// Search the pool for the narrowest tree decomposition whose bags all come
/// from it.
///
/// After the first answer, the widest pieces of it are re-decomposed on their
/// own and their bags added to the pool, which is Tamaki's growth step; the
/// programme is run again over the longer list while there is time for it.
///
/// Returns `None` when the pool is empty, when the search would exceed its
/// [`Limits`], or when `deadline` passes.
pub(crate) fn recombine(
    pool: &BagPool,
    graph: &Graph,
    deadline: Option<Instant>,
) -> Option<TreeDecomposition> {
    if pool.is_empty() || expired(deadline) {
        return None;
    }
    let adjacency = adjacency_lists(graph);
    let mut bags = pool.assemble(graph, deadline);
    if bags.is_empty() {
        return None;
    }
    let mut best = search(&bags, graph, &adjacency, pool.limits, deadline)?;
    for _ in 0..GROWTH_ROUNDS {
        if expired(deadline) || !grow(&mut bags, &best, &adjacency, pool.limits, deadline) {
            break;
        }
        let Some(next) = search(&bags, graph, &adjacency, pool.limits, deadline) else {
            break;
        };
        if next.quality_key() >= best.quality_key() {
            break;
        }
        best = next;
    }
    Some(best)
}

/// One run of the dynamic programme over `bags`.
fn search(
    bags: &[Vec<u32>],
    graph: &Graph,
    adjacency: &[Vec<u32>],
    limits: Limits,
    deadline: Option<Instant>,
) -> Option<TreeDecomposition> {
    let n = graph.num_vertices() as usize;
    let mut search = Search {
        n,
        adjacency,
        limits,
        blocks: Vec::new(),
        index: FxHashMap::default(),
        stored: 0,
        subblocks: vec![Vec::new(); bags.len()],
        collected: 0,
        bag_sizes: bags.iter().map(Vec::len).collect(),
        mark: vec![u32::MAX; n],
        stamp: 0,
        seen: vec![false; n],
    };
    search.collect(bags, deadline)?;
    search.evaluate(deadline)?;
    search.build(bags, graph, deadline)
}

/// Take the widest bags of `answer`, decompose the piece around each of them on
/// its own, and add the bags that come back to `bags`.
///
/// A piece is one bag together with the bags next to it in the tree: small,
/// overlapping, and where the width of the answer is. Decomposing it by MCS-M
/// gives the cliques of a minimal triangulation of that piece, which are the
/// bags the programme could not have assembled from the pool it was given.
/// Returns whether anything new was added.
fn grow(
    bags: &mut Vec<Vec<u32>>,
    answer: &TreeDecomposition,
    adjacency: &[Vec<u32>],
    limits: Limits,
    deadline: Option<Instant>,
) -> bool {
    let width = answer.treewidth();
    let mut widest: Vec<usize> = (0..answer.bags().len())
        .filter(|&index| answer.bags()[index].vertices().len() as u32 > width)
        .collect();
    widest.sort_by_key(|&index| std::cmp::Reverse(answer.bags()[index].vertices().len()));
    widest.truncate(GROWTH_PIECES);
    let known: FxHashSet<&Vec<u32>> = bags.iter().collect();
    let mut fresh: Vec<Vec<u32>> = Vec::new();
    let mut done: Vec<Vec<u32>> = Vec::new();
    let mut stored: usize = bags.iter().map(Vec::len).sum();
    for index in widest {
        if expired(deadline) || bags.len() + fresh.len() >= limits.bags {
            break;
        }
        let mut piece: Vec<u32> = answer.bags()[index].vertices().to_vec();
        for &next in &answer.adjacency()[index] {
            piece.extend_from_slice(answer.bags()[next].vertices());
        }
        piece.sort_unstable();
        piece.dedup();
        if done.contains(&piece) {
            continue;
        }
        done.push(piece.clone());
        for bag in decompose_piece(&piece, adjacency, deadline) {
            if known.contains(&bag) || fresh.contains(&bag) {
                continue;
            }
            if bags.len() + fresh.len() >= limits.bags || stored >= limits.pool_vertices {
                break;
            }
            stored += bag.len();
            fresh.push(bag);
        }
    }
    let added = !fresh.is_empty();
    bags.append(&mut fresh);
    added
}

/// The bags of a minimal triangulation of the subgraph `piece` induces, in the
/// vertex numbering of the whole graph.
fn decompose_piece(
    piece: &[u32],
    adjacency: &[Vec<u32>],
    deadline: Option<Instant>,
) -> Vec<Vec<u32>> {
    let mut position = FxHashMap::default();
    for (local, &vertex) in piece.iter().enumerate() {
        position.insert(vertex, local as u32);
    }
    let mut edges = Vec::new();
    for (local, &vertex) in piece.iter().enumerate() {
        for &next in &adjacency[vertex as usize] {
            if let Some(&other) = position.get(&next)
                && (other as usize) > local
            {
                edges.push((local as u32, other));
            }
        }
    }
    let graph = Graph::new(piece.len() as u32, edges);
    let budget = deadline.map(|deadline| crate::deadline::remaining(deadline) / 4);
    let Ok(decomposition) = crate::elimination::decompose(
        &graph,
        crate::elimination::Order::MinimalTriangulation,
        0,
        budget,
    ) else {
        return Vec::new();
    };
    decomposition
        .bags()
        .iter()
        .map(|bag| {
            let mut vertices: Vec<u32> = bag
                .vertices()
                .iter()
                .map(|&local| piece[local as usize])
                .collect();
            vertices.sort_unstable();
            vertices
        })
        .collect()
}

/// Adjacency lists over `0..num_vertices`.
fn adjacency_lists(graph: &Graph) -> Vec<Vec<u32>> {
    let mut adjacency = vec![Vec::new(); graph.num_vertices() as usize];
    for &(left, right) in graph.edges() {
        adjacency[left as usize].push(right);
        adjacency[right as usize].push(left);
    }
    adjacency
}

/// The components of `G − Ω`, with the vertices of `Ω` on each one's border.
struct Split {
    components: Vec<Vec<u32>>,
    separators: Vec<Vec<u32>>,
}

struct Search<'a> {
    n: usize,
    adjacency: &'a [Vec<u32>],
    limits: Limits,
    blocks: Vec<Block>,
    index: FxHashMap<Vec<u32>, usize>,
    stored: usize,
    /// For each pool bag, the blocks that are the components of `G` less that
    /// bag.
    subblocks: Vec<Vec<usize>>,
    /// How many pool bags were taken in whole. The rest were left out when a
    /// cap was reached, and neither they nor the caps they had begun to
    /// register may be read.
    collected: usize,
    bag_sizes: Vec<usize>,
    /// Stamped membership marks, so a vertex set can be tested without an
    /// array being cleared per query.
    mark: Vec<u32>,
    stamp: u32,
    /// Scratch for the traversal.
    seen: Vec<bool>,
}

impl Search<'_> {
    /// Mark `vertices` and return the stamp that identifies them.
    fn mark_set(&mut self, vertices: &[u32]) -> u32 {
        self.stamp = self.stamp.wrapping_add(1);
        for &vertex in vertices {
            self.mark[vertex as usize] = self.stamp;
        }
        self.stamp
    }

    fn marked(&self, vertex: u32, stamp: u32) -> bool {
        self.mark[vertex as usize] == stamp
    }

    /// The connected components of `G` less `bag`, each with its separator.
    fn split(&mut self, bag: &[u32]) -> Split {
        let removed = self.mark_set(bag);
        self.seen.fill(false);
        for &vertex in bag {
            self.seen[vertex as usize] = true;
        }
        let mut components: Vec<Vec<u32>> = Vec::new();
        let mut separators: Vec<Vec<u32>> = Vec::new();
        let mut stack = Vec::new();
        for start in 0..self.n {
            if self.seen[start] {
                continue;
            }
            self.seen[start] = true;
            stack.push(start as u32);
            let mut component = Vec::new();
            let mut separator = Vec::new();
            while let Some(vertex) = stack.pop() {
                component.push(vertex);
                for &next in &self.adjacency[vertex as usize] {
                    if self.marked(next, removed) {
                        separator.push(next);
                    } else if !self.seen[next as usize] {
                        self.seen[next as usize] = true;
                        stack.push(next);
                    }
                }
            }
            component.sort_unstable();
            separator.sort_unstable();
            separator.dedup();
            components.push(component);
            separators.push(separator);
        }
        Split {
            components,
            separators,
        }
    }

    /// Register a block, or return the one already there. `None` when the
    /// block map is full.
    fn block(&mut self, component: Vec<u32>, separator: Vec<u32>) -> Option<usize> {
        if let Some(&index) = self.index.get(&component) {
            return Some(index);
        }
        let size = component.len() + separator.len();
        if self.stored.saturating_add(size) > self.limits.block_vertices {
            return None;
        }
        self.stored += size;
        let index = self.blocks.len();
        self.index.insert(component.clone(), index);
        self.blocks.push(Block {
            component,
            separator,
            caps: Vec::new(),
            width: u32::MAX,
            best_cap: None,
        });
        Some(index)
    }

    /// Walk the pool: for each bag, the blocks it splits the graph into, and
    /// the block it caps on the far side of each of them.
    fn collect(&mut self, bags: &[Vec<u32>], deadline: Option<Instant>) -> Option<()> {
        for (bag_index, bag) in bags.iter().enumerate() {
            if expired(deadline) {
                return None;
            }
            let split = self.split(bag);
            let mut subblocks = Vec::with_capacity(split.components.len());
            for (component, separator) in split.components.iter().zip(&split.separators) {
                let Some(block) = self.block(component.clone(), separator.clone()) else {
                    return Some(());
                };
                subblocks.push(block);
            }
            self.subblocks[bag_index] = subblocks;
            // Two indexes over the bag, both built once and read once per
            // component: which components each bag vertex borders, and which
            // other bag vertices it is adjacent to.
            let mut borders: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
            for (id, separator) in split.separators.iter().enumerate() {
                for &vertex in separator {
                    borders.entry(vertex).or_default().push(id);
                }
            }
            let inside_bag = self.mark_set(bag);
            let mut bag_neighbours: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
            for &vertex in bag {
                let neighbours: Vec<u32> = self.adjacency[vertex as usize]
                    .iter()
                    .copied()
                    .filter(|&next| self.marked(next, inside_bag))
                    .collect();
                bag_neighbours.insert(vertex, neighbours);
            }
            for position in 0..split.components.len() {
                if expired(deadline) {
                    return None;
                }
                let Some((capped, separator)) =
                    self.capped_block(bag, &split, &borders, &bag_neighbours, position)
                else {
                    continue;
                };
                let Some(index) = self.block(capped, separator) else {
                    return Some(());
                };
                self.blocks[index].caps.push(bag_index);
            }
            self.collected = bag_index + 1;
        }
        Some(())
    }

    /// The component of `G` less the `position`-th component's separator that
    /// holds the rest of `bag`, with that component's own separator. `None`
    /// where the bag is the separator and so caps nothing.
    ///
    /// The far side is what is left of the bag once the separator is taken out,
    /// plus every other component of `G − Ω` that touches it. Nothing further
    /// joins: two components of `G − Ω` share no edge, so a component reached
    /// from one of them would have to be reached through the bag, and the only
    /// bag vertices left are already there.
    ///
    /// Its separator is not the whole of `N(C)`: a vertex there may border `C`
    /// and nothing on the far side. Taking the exact set matters, because a
    /// block's separator is what its parent bag is guaranteed to contain.
    fn capped_block(
        &mut self,
        bag: &[u32],
        split: &Split,
        borders: &FxHashMap<u32, Vec<usize>>,
        bag_neighbours: &FxHashMap<u32, Vec<u32>>,
        position: usize,
    ) -> Option<(Vec<u32>, Vec<u32>)> {
        let border = self.mark_set(&split.separators[position]);
        let rest: Vec<u32> = bag
            .iter()
            .copied()
            .filter(|&vertex| !self.marked(vertex, border))
            .collect();
        if rest.is_empty() {
            return None;
        }
        let mut touched = vec![false; split.components.len()];
        for vertex in &rest {
            for &id in borders.get(vertex).map(Vec::as_slice).unwrap_or(&[]) {
                touched[id] = true;
            }
        }
        let far = self.mark_set(&rest);
        let mut separator = Vec::new();
        for &vertex in &split.separators[position] {
            let adjacent_to_rest = bag_neighbours
                .get(&vertex)
                .is_some_and(|neighbours| neighbours.iter().any(|&next| self.marked(next, far)));
            let borders_far_component = borders
                .get(&vertex)
                .is_some_and(|ids| ids.iter().any(|&id| id != position && touched[id]));
            if adjacent_to_rest || borders_far_component {
                separator.push(vertex);
            }
        }
        let mut capped = rest;
        for (id, component) in split.components.iter().enumerate() {
            if id != position && touched[id] {
                capped.extend_from_slice(component);
            }
        }
        capped.sort_unstable();
        separator.sort_unstable();
        separator.dedup();
        Some((capped, separator))
    }

    /// Settle every block's width, smallest component first.
    fn evaluate(&mut self, deadline: Option<Instant>) -> Option<()> {
        let mut order: Vec<usize> = (0..self.blocks.len()).collect();
        order.sort_by_key(|&index| self.blocks[index].component.len());
        for (step, index) in order.into_iter().enumerate() {
            if step % DEADLINE_STRIDE == 0 && expired(deadline) {
                return None;
            }
            let inside = {
                let component = &self.blocks[index].component;
                self.stamp = self.stamp.wrapping_add(1);
                let stamp = self.stamp;
                for &vertex in component {
                    self.mark[vertex as usize] = stamp;
                }
                stamp
            };
            // Everything in one bag, which is always available and always
            // valid, against the best cap. A cap that only ties is still
            // preferred: it says the same width in smaller bags.
            let whole = (self.blocks[index].component.len() + self.blocks[index].separator.len())
                .saturating_sub(1) as u32;
            let mut best: Option<(u32, usize)> = None;
            for position in 0..self.blocks[index].caps.len() {
                let cap = self.blocks[index].caps[position];
                if cap >= self.collected {
                    continue;
                }
                let Some(candidate) = self.cap_width(cap, index, inside) else {
                    continue;
                };
                if best.is_none_or(|(width, _)| candidate < width) {
                    best = Some((candidate, cap));
                }
            }
            let (width, best_cap) = match best {
                Some((width, cap)) if width <= whole => (width, Some(cap)),
                _ => (whole, None),
            };
            self.blocks[index].width = width;
            self.blocks[index].best_cap = best_cap;
        }
        Some(())
    }

    /// What putting pool bag `cap` at the top of block `block` costs, or
    /// `None` where a sub-block it leaves has no width yet.
    fn cap_width(&self, cap: usize, block: usize, inside: u32) -> Option<u32> {
        let mut width = self.bag_sizes[cap].saturating_sub(1) as u32;
        for &sub in &self.subblocks[cap] {
            if sub == block {
                // The cap left the block it caps whole, so it is not a cap at
                // all; using it would be circular.
                return None;
            }
            let other = &self.blocks[sub];
            // A sub-block lies wholly inside the component or wholly outside
            // it, so one vertex settles it.
            let representative = *other.component.first()?;
            if !self.marked(representative, inside) {
                continue;
            }
            if other.width == u32::MAX {
                return None;
            }
            width = width.max(other.width);
        }
        Some(width)
    }

    /// Assemble the narrowest decomposition the settled widths describe.
    fn build(
        &mut self,
        bags_pool: &[Vec<u32>],
        graph: &Graph,
        deadline: Option<Instant>,
    ) -> Option<TreeDecomposition> {
        if expired(deadline) {
            return None;
        }
        let mut best: Option<(u32, usize)> = None;
        for (index, bag) in bags_pool.iter().enumerate().take(self.collected) {
            let mut width = bag.len().saturating_sub(1) as u32;
            let mut usable = true;
            for &sub in &self.subblocks[index] {
                if self.blocks[sub].width == u32::MAX {
                    usable = false;
                    break;
                }
                width = width.max(self.blocks[sub].width);
            }
            if usable && best.is_none_or(|(best_width, _)| width < best_width) {
                best = Some((width, index));
            }
        }
        let (_, root) = best?;
        let mut bags: Vec<Vec<u32>> = vec![bags_pool[root].clone()];
        let mut edges: Vec<(usize, usize)> = Vec::new();
        let mut pending: Vec<(usize, usize)> =
            self.subblocks[root].iter().map(|&b| (b, 0usize)).collect();
        while let Some((block, parent)) = pending.pop() {
            if expired(deadline) {
                return None;
            }
            let position = bags.len();
            match self.blocks[block].best_cap {
                Some(cap) => {
                    bags.push(bags_pool[cap].clone());
                    let component = std::mem::take(&mut self.blocks[block].component);
                    let inside = self.mark_set(&component);
                    self.blocks[block].component = component;
                    for &sub in &self.subblocks[cap] {
                        let representative = *self.blocks[sub].component.first()?;
                        if self.marked(representative, inside) {
                            pending.push((sub, position));
                        }
                    }
                }
                None => {
                    // No cap was worth it, so the block goes into one bag: its
                    // component and its separator together.
                    let mut bag = self.blocks[block].component.clone();
                    bag.extend_from_slice(&self.blocks[block].separator);
                    bag.sort_unstable();
                    bag.dedup();
                    bags.push(bag);
                }
            }
            edges.push((parent, position));
        }
        TreeDecomposition::new(graph, bags, edges).ok()
    }
}
