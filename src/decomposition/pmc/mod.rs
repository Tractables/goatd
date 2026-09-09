//! Build a decomposition out of bags taken from several others.
//!
//! Every construction here produces a whole tree decomposition and the caller
//! keeps the narrowest. That throws away the good bags of every other one. This
//! module keeps them instead: it holds the best few decompositions a run
//! produced, pools their bags, and searches that pool for the narrowest tree
//! decomposition whose every bag comes from it.
//!
//! The pool holds the best decomposition of every stage the run has, rather
//! than the best few decompositions overall: what the search needs from a
//! candidate is a tree shaped differently from the others, and the narrowest
//! four are usually four near copies of one another. Where they do not all fit,
//! the bags are shared out — the narrowest tree twice the share of the rest,
//! and each gives up its narrowest bags first. Each is minimalised on the way
//! in where there is time for it, so the bags pooled are the cliques of a
//! minimal triangulation rather than whatever the elimination left.
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

mod merge;
mod sets;

use sets::{Adjacency, Scratch, Split, VertexSet};

pub(crate) use merge::merge_loop;

/// Build a decomposition by the merge loop alone: an initial answer of several
/// randomised minimal triangulations, then independent answers built, improved
/// and merged into it until `budget` runs out.
///
/// This is the portfolio's merge stage standing on its own, without a
/// portfolio to start it off. Where the budget runs out before the programme
/// settles anything, a single min-fill decomposition comes back instead, so
/// the call always answers.
///
/// # Errors
///
/// Returns an error if the budget is too large to represent as a deadline, or
/// if the fallback elimination fails.
pub fn decompose_by_merging(
    graph: &Graph,
    seed: u64,
    budget: Option<std::time::Duration>,
) -> Result<TreeDecomposition, crate::Error> {
    let deadline = budget
        .map(|budget| crate::deadline::checked(crate::meter::now(), budget, "merge loop"))
        .transpose()?;
    if let Some(found) = merge_loop(graph, None, seed, Limits::standard(), deadline) {
        return Ok(found);
    }
    crate::elimination::decompose(graph, crate::elimination::Order::MinFill, seed, budget)
}

#[cfg(test)]
mod tests;

/// Distinct bags the pool may hold.
const MAX_POOL_BAGS: usize = 4_000;
/// Vertex ids the pool may hold.
const MAX_POOL_VERTICES: usize = 1_000_000;
/// Vertex ids the block map may hold.
const MAX_BLOCK_VERTICES: usize = 32_000_000;
/// Stages the pool keeps a decomposition from.
const POOL_SLOTS: usize = 16;
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
    slots: usize,
}

impl Limits {
    /// The caps every graph is searched under.
    pub(crate) fn standard() -> Self {
        Self {
            pool_vertices: MAX_POOL_VERTICES,
            block_vertices: MAX_BLOCK_VERTICES,
            bags: MAX_POOL_BAGS,
            slots: POOL_SLOTS,
        }
    }

    /// How many stages the pool keeps a decomposition from.
    pub(crate) fn slots(&self) -> usize {
        self.slots
    }

    /// The bags the pool may hold in all.
    pub(crate) fn bags(&self) -> usize {
        self.bags
    }
}

/// The best decomposition each stage of a run produced, kept whole so their
/// bags can be pooled together.
///
/// The pool is keyed by the stage that produced the decomposition, not by width
/// alone, because what the search needs from a candidate is a tree shaped
/// differently from the others: the sampled draws, the diverse passes,
/// FlowCutter and nested dissection cut the graph in different places, and a
/// pool of the four narrowest trees is usually four near copies of one of them.
/// Every slot that produced anything is in the pool, and the bags are shared
/// out between them when they do not all fit, with the narrowest given twice
/// the share of the rest.
pub(crate) struct BagPool {
    kept: Vec<Kept>,
    limits: Limits,
}

struct Kept {
    slot: u32,
    decomposition: TreeDecomposition,
}

impl BagPool {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            kept: Vec::new(),
            limits,
        }
    }

    /// Offer one decomposition, produced by the stage `slot`.
    pub(crate) fn absorb(&mut self, decomposition: &TreeDecomposition, slot: u32) {
        let key = decomposition.quality_key();
        if let Some(held) = self.kept.iter_mut().find(|held| held.slot == slot) {
            if key < held.decomposition.quality_key() {
                held.decomposition = decomposition.clone();
            }
            return;
        }
        if self.kept.len() < self.limits.slots {
            self.kept.push(Kept {
                slot,
                decomposition: decomposition.clone(),
            });
            return;
        }
        // Full, and this is a stage the pool has nothing from. It takes the
        // place of the widest slot held, so a late stage is not shut out by
        // the order the run happens to produce its candidates in.
        let Some(widest) = self
            .kept
            .iter()
            .enumerate()
            .max_by_key(|(_, held)| held.decomposition.quality_key())
            .map(|(index, _)| index)
        else {
            return;
        };
        if key < self.kept[widest].decomposition.quality_key() {
            self.kept[widest] = Kept {
                slot,
                decomposition: decomposition.clone(),
            };
        }
    }

    /// The bags of everything kept, narrowest decomposition first, each one
    /// minimalised where there is time for it so its bags are the cliques of a
    /// minimal triangulation rather than whatever the elimination left.
    ///
    /// Where the pool cannot hold everything, each kept decomposition gets a
    /// share of it — the narrowest two shares, the rest one each — and gives up
    /// its narrowest bags first, since the width of a decomposition is in its
    /// widest bags and those are what the search is being asked to beat. What
    /// one of them leaves unused is filled from the others afterwards.
    fn assemble(
        &self,
        graph: &Graph,
        adjacency: &Adjacency,
        deadline: Option<Instant>,
    ) -> Vec<VertexSet> {
        let mut sources: Vec<TreeDecomposition> = Vec::new();
        let mut order: Vec<&Kept> = self.kept.iter().collect();
        order.sort_by_key(|held| held.decomposition.quality_key());
        for held in order {
            if expired(deadline) {
                break;
            }
            let kept = &held.decomposition;
            sources.push(super::minimalize_at(kept.clone(), graph, deadline));
        }
        let shares = sources.len() + 1;
        let mut bags: Vec<VertexSet> = Vec::new();
        let mut seen: FxHashSet<VertexSet> = FxHashSet::default();
        let mut stored = 0usize;
        let mut widest: Vec<Vec<usize>> = sources
            .iter()
            .map(|source| {
                let mut indices: Vec<usize> = (0..source.bags().len()).collect();
                indices
                    .sort_by_key(|&index| std::cmp::Reverse(source.bags()[index].vertices().len()));
                indices
            })
            .collect();
        for round in 0..2 {
            for (rank, source) in sources.iter().enumerate() {
                let share = if rank == 0 { 2 } else { 1 };
                let quota = if round == 0 {
                    (self.limits.bags * share / shares).max(1)
                } else {
                    self.limits.bags
                };
                let mut taken = 0;
                let mut left = Vec::new();
                for index in std::mem::take(&mut widest[rank]) {
                    if taken >= quota
                        || bags.len() >= self.limits.bags
                        || stored >= self.limits.pool_vertices
                    {
                        left.push(index);
                        continue;
                    }
                    let bag = adjacency.set_of(source.bags()[index].vertices());
                    taken += 1;
                    if seen.contains(&bag) {
                        continue;
                    }
                    stored += bag.len();
                    seen.insert(bag.clone());
                    bags.push(bag);
                }
                widest[rank] = left;
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
    component: VertexSet,
    separator: VertexSet,
    /// The lowest vertex of the component, which settles whether the component
    /// lies inside another one: two blocks are nested or disjoint.
    representative: u32,
    /// How many vertices the component and the separator hold, kept because
    /// the evaluation order and the caps read them once per block.
    component_size: usize,
    separator_size: usize,
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
    let adjacency = Adjacency::of(graph)?;
    let mut bags = pool.assemble(graph, &adjacency, deadline);
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
    bags: &[VertexSet],
    graph: &Graph,
    adjacency: &Adjacency,
    limits: Limits,
    deadline: Option<Instant>,
) -> Option<TreeDecomposition> {
    let mut search = Search {
        limits,
        blocks: Vec::new(),
        index: FxHashMap::default(),
        stored: 0,
        subblocks: vec![Vec::new(); bags.len()],
        collected: 0,
        bag_sizes: bags.iter().map(VertexSet::len).collect(),
        scratch: Scratch::new(adjacency),
        adjacency,
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
    bags: &mut Vec<VertexSet>,
    answer: &TreeDecomposition,
    adjacency: &Adjacency,
    limits: Limits,
    deadline: Option<Instant>,
) -> bool {
    let width = answer.treewidth();
    let mut widest: Vec<usize> = (0..answer.bags().len())
        .filter(|&index| answer.bags()[index].vertices().len() as u32 > width)
        .collect();
    widest.sort_by_key(|&index| std::cmp::Reverse(answer.bags()[index].vertices().len()));
    widest.truncate(GROWTH_PIECES);
    let known: FxHashSet<&VertexSet> = bags.iter().collect();
    let mut fresh: Vec<VertexSet> = Vec::new();
    let mut done: Vec<VertexSet> = Vec::new();
    let mut stored: usize = bags.iter().map(VertexSet::len).sum();
    for index in widest {
        if expired(deadline) || bags.len() + fresh.len() >= limits.bags {
            break;
        }
        let mut piece = adjacency.set_of(answer.bags()[index].vertices());
        for &next in &answer.adjacency()[index] {
            for &vertex in answer.bags()[next].vertices() {
                piece.insert(vertex);
            }
        }
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
    piece: &VertexSet,
    adjacency: &Adjacency,
    deadline: Option<Instant>,
) -> Vec<VertexSet> {
    let vertices = piece.to_vec();
    let mut edges = Vec::new();
    for (local, &vertex) in vertices.iter().enumerate() {
        for (other, &next) in vertices.iter().enumerate().skip(local + 1) {
            if adjacency.adjacent(vertex, next) {
                edges.push((local as u32, other as u32));
            }
        }
    }
    let graph = Graph::new(vertices.len() as u32, edges);
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
            let mut set = adjacency.empty_set();
            for &local in bag.vertices() {
                set.insert(vertices[local as usize]);
            }
            set
        })
        .collect()
}

/// The components of `G` less the `position`-th component's separator that the
/// rest of `bag` reaches, with that component's own separator. `None` where the
/// bag is the separator and so caps nothing.
///
/// The far side is what is left of the bag once the separator is taken out,
/// plus every other component of `G − Ω` that touches it. Nothing further
/// joins: two components of `G − Ω` share no edge, so a component reached from
/// one of them would have to be reached through the bag, and the only bag
/// vertices left are already there.
///
/// Its separator is not the whole of `N(C)`: a vertex there may border `C` and
/// nothing on the far side. Taking the exact set matters, because a block's
/// separator is what its parent bag is guaranteed to contain.
fn capped_block(
    adjacency: &Adjacency,
    scratch: &mut Scratch,
    bag: &VertexSet,
    split: &Split,
    position: usize,
) -> Option<(VertexSet, VertexSet)> {
    let mut rest = std::mem::take(&mut scratch.held);
    rest.copy_from(bag);
    rest.subtract(&split.borders[position]);
    if rest.is_empty() {
        scratch.held = rest;
        return None;
    }
    let mut capped = rest.clone();
    // What the far side reaches: the neighbours of the rest of the bag, and
    // the borders of the components that the rest touches.
    let mut reach = std::mem::take(&mut scratch.reach);
    reach.clear();
    for (id, component) in split.components.iter().enumerate() {
        if id != position && split.borders[id].intersects(&rest) {
            capped.union_with(component);
            reach.union_with(&split.borders[id]);
        }
    }
    for vertex in rest.iter() {
        reach.union_row(adjacency.row(vertex));
    }
    let mut separator = split.borders[position].clone();
    separator.intersect_with(&reach);
    scratch.held = rest;
    scratch.reach = reach;
    Some((capped, separator))
}

struct Search<'a> {
    adjacency: &'a Adjacency,
    limits: Limits,
    blocks: Vec<Block>,
    index: FxHashMap<VertexSet, usize>,
    stored: usize,
    /// For each pool bag, the blocks that are the components of `G` less that
    /// bag.
    subblocks: Vec<Vec<usize>>,
    /// How many pool bags were taken in whole. The rest were left out when a
    /// cap was reached, and neither they nor the caps they had begun to
    /// register may be read.
    collected: usize,
    bag_sizes: Vec<usize>,
    /// The sets the traversals reuse.
    scratch: Scratch,
}

impl Search<'_> {
    /// Register a block, or return the one already there. `None` when the
    /// block map is full.
    fn block(&mut self, component: VertexSet, separator: VertexSet) -> Option<usize> {
        if let Some(&index) = self.index.get(&component) {
            return Some(index);
        }
        let component_size = component.len();
        let separator_size = separator.len();
        let size = component_size + separator_size;
        if self.stored.saturating_add(size) > self.limits.block_vertices {
            return None;
        }
        self.stored += size;
        let index = self.blocks.len();
        let representative = component.first()?;
        self.index.insert(component.clone(), index);
        self.blocks.push(Block {
            component,
            separator,
            representative,
            component_size,
            separator_size,
            caps: Vec::new(),
            width: u32::MAX,
            best_cap: None,
        });
        Some(index)
    }

    /// Walk the pool: for each bag, the blocks it splits the graph into, and
    /// the block it caps on the far side of each of them.
    fn collect(&mut self, bags: &[VertexSet], deadline: Option<Instant>) -> Option<()> {
        for (bag_index, bag) in bags.iter().enumerate() {
            if expired(deadline) {
                return None;
            }
            let split = self.adjacency.split(bag, &mut self.scratch);
            let mut subblocks = Vec::with_capacity(split.components.len());
            for (component, border) in split.components.iter().zip(&split.borders) {
                let Some(block) = self.block(component.clone(), border.clone()) else {
                    return Some(());
                };
                subblocks.push(block);
            }
            self.subblocks[bag_index] = subblocks;
            for position in 0..split.components.len() {
                if expired(deadline) {
                    return None;
                }
                let Some((capped, separator)) =
                    capped_block(self.adjacency, &mut self.scratch, bag, &split, position)
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

    /// Settle every block's width, smallest component first.
    fn evaluate(&mut self, deadline: Option<Instant>) -> Option<()> {
        let mut order: Vec<usize> = (0..self.blocks.len()).collect();
        order.sort_by_key(|&index| self.blocks[index].component_size);
        for (step, index) in order.into_iter().enumerate() {
            if step % DEADLINE_STRIDE == 0 && expired(deadline) {
                return None;
            }
            // The component is taken out of the block for as long as the
            // caps are read, so that the widths already settled can be read
            // beside it.
            let inside = std::mem::take(&mut self.blocks[index].component);
            // Everything in one bag, which is always available and always
            // valid, against the best cap. A cap that only ties is still
            // preferred: it says the same width in smaller bags.
            let whole = (self.blocks[index].component_size + self.blocks[index].separator_size)
                .saturating_sub(1) as u32;
            let mut best: Option<(u32, usize)> = None;
            for position in 0..self.blocks[index].caps.len() {
                let cap = self.blocks[index].caps[position];
                if cap >= self.collected {
                    continue;
                }
                let Some(candidate) = self.cap_width(cap, index, &inside) else {
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
            self.blocks[index].component = inside;
            self.blocks[index].width = width;
            self.blocks[index].best_cap = best_cap;
        }
        Some(())
    }

    /// What putting pool bag `cap` at the top of block `block` costs, or
    /// `None` where a sub-block it leaves has no width yet.
    fn cap_width(&self, cap: usize, block: usize, inside: &VertexSet) -> Option<u32> {
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
            if !inside.contains(other.representative) {
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
        bags_pool: &[VertexSet],
        graph: &Graph,
        deadline: Option<Instant>,
    ) -> Option<TreeDecomposition> {
        if expired(deadline) {
            return None;
        }
        let mut best: Option<(u32, usize)> = None;
        for index in 0..bags_pool.len().min(self.collected) {
            let mut width = self.bag_sizes[index].saturating_sub(1) as u32;
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
        let mut bags: Vec<Vec<u32>> = vec![bags_pool[root].to_vec()];
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
                    bags.push(bags_pool[cap].to_vec());
                    let inside = std::mem::take(&mut self.blocks[block].component);
                    for &sub in &self.subblocks[cap] {
                        if inside.contains(self.blocks[sub].representative) {
                            pending.push((sub, position));
                        }
                    }
                    self.blocks[block].component = inside;
                }
                None => {
                    // No cap was worth it, so the block goes into one bag: its
                    // component and its separator together.
                    let mut bag = self.blocks[block].component.clone();
                    bag.union_with(&self.blocks[block].separator);
                    bags.push(bag.to_vec());
                }
            }
            edges.push((parent, position));
        }
        TreeDecomposition::new(graph, bags, edges).ok()
    }
}
