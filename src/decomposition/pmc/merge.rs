//! Improve a decomposition by merging it with an independent one.
//!
//! This is the improvement loop of Tamaki, "Heuristic computation of exact
//! treewidth", 2022, which is the successor of the note "A heuristic use of
//! dynamic programming to upperbound treewidth", 2019. Both keep a *list of
//! bags* as the current answer rather than a tree, and read the width of the
//! list with the restricted Bouchitté–Todinca programme of [`super`]. The
//! width of a list `Π` is written `w(Π)` below: the narrowest tree
//! decomposition every bag of which is in `Π`.
//!
//! The loop is the part of that construction the recombination stage does not
//! have. The stage pools the bags of the trees a run already built and searches
//! them. Here a *second* answer is built from scratch, improved on its own
//! until it is no wider than the first, and only then merged in — so the bags
//! that arrive were never near the first answer, and the recursion is a chain
//! of independent searches rather than a walk outwards from one tree.
//!
//! Three procedures, following the paper:
//!
//! - **an initial answer.** Several randomised min-fill draws, each minimalised
//!   so its bags are the cliques of a minimal triangulation; keep the narrowest.
//!   The paper uses a randomised minimum-average-fill triangulation made
//!   minimal by the method of Berry, Heggernes and Simonet; the sampled
//!   min-fill this library already has, minimalised the same way, is the same
//!   thing built out of what is here.
//! - **merge `Π` with `Ω`.** Pick a bag `X ∈ Π` at random and let `C` be the
//!   largest component of `G − X`. For every `Y ∈ Ω` inside `N[C]` and no wider
//!   than `w(Π)`, let `D` be the component of `G − Y` that holds `X` in its
//!   closed neighbourhood — the pair `X, Y` has one exactly when neither
//!   crosses the other — and let `U = N[C] ∩ N[D]`. Take the smallest such `U`
//!   first, build the local graph on it, triangulate that minimally, and where
//!   the piece comes back no wider than `w(Π)` add its cliques. The merged list
//!   is `Π ∪ Ω` and everything so added, which admits trees that neither list
//!   admits on its own: a tree can now take some bags from `Π`, some from `Ω`,
//!   and the added cliques to join the two.
//! - **improve `Π`.** Build an initial answer `Ω`; while `w(Ω) > w(Π)`, improve
//!   `Ω` by the same procedure; then merge. The recursion is the reason the
//!   side answer is worth merging: it is not a variation of `Π` but a
//!   competitor of it.
//!
//! The paper's loop runs until the width meets a lower bound. Here it runs to a
//! deadline, and a level of the recursion takes half of what is left when it
//! starts, so the levels below it cannot spend the window on their own. Where
//! the deadline stops a side answer above `w(Π)` it is merged anyway: its bags
//! are still bags of a triangulation the first answer does not have.
//!
//! Two things the paper does that this does not. It solves a piece of at most
//! 60 vertices exactly, where this triangulates every piece greedily; and it
//! keeps improving after the width drops, where the caller here gets one
//! answer per stage of a portfolio.

use std::time::Instant;

use rustc_hash::FxHashSet;

use super::sets::{Adjacency, Scratch, VertexSet};
use super::{Limits, search};
use crate::deadline::{expired, remaining};
use crate::rng::{SEED_OFFSET, Xorshift64};
use crate::{Graph, TreeDecomposition};

/// Draws the initial answer is built from, and the focuses tried per merge.
/// Both are the paper's (`N_INITIAL_GREEDY` and `N_TRY`).
const INITIAL_DRAWS: usize = 10;
const FOCUSES_PER_MERGE: usize = 50;
/// How deep the chain of side answers may go. The paper's recursion ends when
/// a side answer reaches the width above it; this bounds the depth as well,
/// since every level costs a run of the programme.
const MAX_DEPTH: usize = 4;
/// Share of the time left that a level of the recursion may spend, so the
/// level above it still has time to merge what it is handed.
const LEVEL_TIME_SHARE: u32 = 2;
/// Candidate partners looked at between two reads of the clock.
const DEADLINE_STRIDE: usize = 16;

/// A list of bags and the narrowest tree the programme builds out of it.
struct Answer {
    bags: Vec<VertexSet>,
    tree: TreeDecomposition,
}

impl Answer {
    fn width(&self) -> u32 {
        self.tree.treewidth()
    }
}

/// What every level of the recursion shares. The local stage of [`super`]
/// builds one of these too, since it re-triangulates between two bags the same
/// way.
pub(super) struct Loop<'a> {
    graph: &'a Graph,
    adjacency: &'a Adjacency,
    limits: Limits,
    rng: Xorshift64,
    /// Uniform sampling weights for the randomised draws, one per vertex.
    weights: Vec<u32>,
}

/// Improve `start` by merging independent answers into it until `deadline`.
///
/// `start` is where the list begins: its bags, minimalised where there is time
/// for it. With `None` the construction stands on its own and begins from its
/// own initial answer, which is how the paper's algorithm starts.
///
/// Returns the narrowest tree the programme built, or `None` where the graph
/// has no vertices, the deadline passes before an answer is settled, or the
/// search would exceed its [`Limits`].
pub(crate) fn merge_loop(
    graph: &Graph,
    start: Option<&TreeDecomposition>,
    seed: u64,
    limits: Limits,
    deadline: Option<Instant>,
) -> Option<TreeDecomposition> {
    if graph.num_vertices() == 0 || expired(deadline) {
        return None;
    }
    let adjacency = Adjacency::of(graph)?;
    let mut state = Loop::new(graph, &adjacency, limits, seed);
    let mut answer = match start {
        Some(start) => {
            let minimalised = crate::decomposition::minimalize_at(start.clone(), graph, deadline);
            state.settle(bags_of(&minimalised, &adjacency), deadline)?
        }
        None => state.initial(deadline)?,
    };
    let mut scratch = Scratch::new(&adjacency);
    // Every round keeps what it merged in, whether or not the width moved. The
    // list only grows, so the programme over it never reads a wider tree than
    // the round before, and one merge on its own rarely lowers anything: what
    // lowers the width is the bags of several of them together. The round that
    // adds nothing at all is the one worth stopping on.
    while !expired(deadline) {
        let held = answer.bags.len();
        let Some(next) = state.improve(&answer, 0, &mut scratch, deadline) else {
            break;
        };
        answer = next;
        // At width k the list drops every bag of more than k + 2 vertices: no
        // tree of width k or less has one, and this is what keeps it from
        // growing without bound as the merges accumulate.
        let room = answer.width() as usize + 2;
        answer.bags.retain(|bag| bag.len() <= room);
        if answer.bags.len() <= held {
            break;
        }
    }
    Some(answer.tree)
}

/// The bags of a decomposition, sorted and without repeats.
pub(super) fn bags_of(decomposition: &TreeDecomposition, adjacency: &Adjacency) -> Vec<VertexSet> {
    let mut bags: Vec<Vec<u32>> = decomposition
        .bags()
        .iter()
        .map(|bag| {
            let mut vertices = bag.vertices().to_vec();
            vertices.sort_unstable();
            vertices.dedup();
            vertices
        })
        .collect();
    bags.sort();
    bags.dedup();
    bags.iter().map(|bag| adjacency.set_of(bag)).collect()
}

impl<'a> Loop<'a> {
    /// The graph, its rows of words, the caps the search runs under and the
    /// random source the draws and the choice of bag read.
    pub(super) fn new(
        graph: &'a Graph,
        adjacency: &'a Adjacency,
        limits: Limits,
        seed: u64,
    ) -> Self {
        Self {
            graph,
            adjacency,
            limits,
            rng: Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET)),
            weights: vec![0; graph.num_vertices() as usize],
        }
    }
}

impl Loop<'_> {
    /// Run the programme over `bags` and keep both.
    fn settle(&self, bags: Vec<VertexSet>, deadline: Option<Instant>) -> Option<Answer> {
        let tree = search(&bags, self.graph, self.adjacency, self.limits, deadline)?;
        Some(Answer { bags, tree })
    }

    /// An answer built from scratch: several randomised min-fill draws, each
    /// minimalised, the narrowest kept.
    ///
    /// The draws share a share of the time left, so a graph where one draw is
    /// slow takes fewer of them rather than the whole window.
    fn initial(&mut self, deadline: Option<Instant>) -> Option<Answer> {
        let until = share(deadline);
        let each = until.map(|until| remaining(until) / INITIAL_DRAWS as u32);
        let mut best: Option<TreeDecomposition> = None;
        for _ in 0..INITIAL_DRAWS {
            if expired(until) {
                break;
            }
            let seed = self.rng.next_u64();
            let order = crate::elimination::Order::MinFillSampled {
                weights: &self.weights,
            };
            let Ok(drawn) = crate::elimination::decompose(self.graph, order, seed, each) else {
                break;
            };
            let drawn = crate::decomposition::minimalize_at(drawn, self.graph, until);
            if best
                .as_ref()
                .is_none_or(|best| drawn.quality_key() < best.quality_key())
            {
                best = Some(drawn);
            }
        }
        self.settle(bags_of(&best?, self.adjacency), deadline)
    }

    /// One improvement of `answer`: build a side answer, improve it until it is
    /// no wider, merge it in and run the programme over the merged list.
    fn improve(
        &mut self,
        answer: &Answer,
        depth: usize,
        scratch: &mut Scratch,
        deadline: Option<Instant>,
    ) -> Option<Answer> {
        let until = share(deadline);
        let mut side = self.initial(until)?;
        while side.width() > answer.width() && depth + 1 < MAX_DEPTH && !expired(until) {
            let held = side.bags.len();
            let Some(better) = self.improve(&side, depth + 1, scratch, until) else {
                break;
            };
            side = better;
            if side.bags.len() <= held {
                break;
            }
        }
        let merged = self.merge(answer, &side, scratch, deadline);
        self.settle(merged, deadline)
    }

    /// The merged list: both lists, and the cliques of a minimal triangulation
    /// of each focus the two answers pick out between them.
    fn merge(
        &mut self,
        answer: &Answer,
        side: &Answer,
        scratch: &mut Scratch,
        deadline: Option<Instant>,
    ) -> Vec<VertexSet> {
        let width = answer.width();
        let mut bags = answer.bags.clone();
        let mut held: FxHashSet<VertexSet> = bags.iter().cloned().collect();
        for bag in &side.bags {
            if held.insert(bag.clone()) {
                bags.push(bag.clone());
            }
        }
        let Some(focuses) = self.focuses(answer, side, scratch, deadline) else {
            return bags;
        };
        let mut stored: usize = bags.iter().map(VertexSet::len).sum();
        for focus in focuses {
            if expired(deadline) || bags.len() >= self.limits.bags {
                break;
            }
            for clique in self.triangulate(&focus, width, scratch, deadline) {
                if bags.len() >= self.limits.bags
                    || stored.saturating_add(clique.len()) > self.limits.pool_vertices
                {
                    break;
                }
                if held.insert(clique.clone()) {
                    stored += clique.len();
                    bags.push(clique);
                }
            }
        }
        bags
    }

    /// The vertex sets to re-triangulate, smallest first.
    ///
    /// One bag `X` of `answer` is drawn at random and `C` is the largest
    /// component of `G − X`. A partner `Y` of the side answer has to lie inside
    /// `N[C]` and be no wider than the answer already is — the first so that
    /// the two do not cross, the second so that the tree the merge admits can
    /// be narrower than the one there is. The focus of the pair is
    /// `N[C] ∩ N[D]`, where `D` is the component of `G − Y` that holds `X`.
    fn focuses(
        &mut self,
        answer: &Answer,
        side: &Answer,
        scratch: &mut Scratch,
        deadline: Option<Instant>,
    ) -> Option<Vec<Vec<u32>>> {
        let width = answer.width() as usize;
        let pick = (self.rng.next_u64() % answer.bags.len() as u64) as usize;
        let chosen = answer.bags[pick].clone();
        Some(self.focuses_from(&chosen, &side.bags, width, scratch, deadline))
    }

    /// The vertex sets to re-triangulate for one bag `chosen` against the list
    /// `partners`, smallest first.
    ///
    /// `C` is the largest component of `G − chosen`; a partner has to lie
    /// inside `N[C]` and hold at most `width` vertices.
    pub(super) fn focuses_from(
        &self,
        chosen: &VertexSet,
        partners: &[VertexSet],
        width: usize,
        scratch: &mut Scratch,
        deadline: Option<Instant>,
    ) -> Vec<Vec<u32>> {
        let split = self.adjacency.split(chosen, scratch);
        let Some(largest) = split
            .components
            .iter()
            .zip(&split.borders)
            .max_by_key(|(component, _)| component.len())
        else {
            return Vec::new();
        };
        let mut inside = largest.0.clone();
        inside.union_with(largest.1);
        let mut found: Vec<Vec<u32>> = Vec::new();
        for (index, partner) in partners.iter().enumerate() {
            if index % DEADLINE_STRIDE == 0 && expired(deadline) {
                break;
            }
            if partner.len() > width || !partner.is_subset(&inside) {
                continue;
            }
            let across = self.adjacency.split(partner, scratch);
            let mut focus = None;
            for (component, border) in across.components.iter().zip(&across.borders) {
                let mut closed = component.clone();
                closed.union_with(border);
                if chosen.is_subset(&closed) {
                    closed.intersect_with(&inside);
                    focus = Some(closed);
                    break;
                }
            }
            let Some(focus) = focus else {
                continue;
            };
            if focus.len() > 1 {
                found.push(focus.to_vec());
            }
        }
        found.sort_by(|one, other| one.len().cmp(&other.len()).then_with(|| one.cmp(other)));
        found.dedup();
        found.truncate(FOCUSES_PER_MERGE);
        found
    }

    /// The cliques of a minimal triangulation of the local graph on `focus`,
    /// where that triangulation is no wider than `width`, and only the ones
    /// that are potential maximal cliques of the whole graph.
    ///
    /// The local graph is the subgraph on `focus` with the neighbourhood of
    /// every component outside it filled into a clique. Filling those is what
    /// makes a triangulation of the piece extend to one of the graph, so a
    /// clique of it is a potential maximal clique of the graph or a minimal
    /// separator of it; the test drops the separators, which cost the
    /// programme a pass over the graph and split nothing.
    fn triangulate(
        &self,
        focus: &[u32],
        width: u32,
        scratch: &mut Scratch,
        deadline: Option<Instant>,
    ) -> Vec<VertexSet> {
        let Some((local, order)) = self.local_graph(focus, scratch) else {
            return Vec::new();
        };
        let budget = deadline.map(|deadline| remaining(deadline) / LEVEL_TIME_SHARE);
        let triangulated = crate::elimination::decompose(
            &local,
            crate::elimination::Order::MinimalTriangulation,
            0,
            budget,
        );
        let Ok(triangulated) = triangulated else {
            return Vec::new();
        };
        if triangulated.treewidth() > width {
            return Vec::new();
        }
        self.cliques_of(&triangulated, &order, width, scratch)
    }

    /// The same piece triangulated harder: `draws` minimal triangulations of
    /// it, then the restricted programme over the bags of all of them
    /// together, and the cliques of whichever came out narrowest.
    ///
    /// The paper triangulates a piece of at most sixty vertices exactly, which
    /// is what makes its step worth taking on the pieces that matter. This
    /// stands in for that: several draws differ on a piece this size, and the
    /// programme over their bags together is at least as narrow as the best of
    /// them and often narrower. It is only worth the extra runs on a small
    /// piece, so the caller asks for one draw on the rest.
    pub(super) fn triangulate_pooled(
        &mut self,
        focus: &[u32],
        width: u32,
        draws: usize,
        scratch: &mut Scratch,
        deadline: Option<Instant>,
    ) -> Vec<VertexSet> {
        let Some((local, order)) = self.local_graph(focus, scratch) else {
            return Vec::new();
        };
        let Some(rows) = Adjacency::of(&local) else {
            return Vec::new();
        };
        let weights = vec![0u32; local.num_vertices() as usize];
        let mut pooled: Vec<VertexSet> = Vec::new();
        let mut seen: FxHashSet<VertexSet> = FxHashSet::default();
        let mut best: Option<TreeDecomposition> = None;
        for draw in 0..draws.max(1) {
            if expired(deadline) {
                break;
            }
            let budget = deadline.map(|deadline| remaining(deadline) / LEVEL_TIME_SHARE);
            // The first draw is the deterministic one, so a piece that one
            // triangulation settles is settled the same way here as in the
            // merge loop; the rest are randomised and made minimal.
            let (choice, seed) = if draw == 0 {
                (crate::elimination::Order::MinimalTriangulation, 0)
            } else {
                (
                    crate::elimination::Order::MinFillSampled { weights: &weights },
                    self.rng.next_u64(),
                )
            };
            let Ok(drawn) = crate::elimination::decompose(&local, choice, seed, budget) else {
                continue;
            };
            let drawn = if crate::decomposition::minimalize_fits(&drawn, &local, deadline) {
                crate::decomposition::minimalize_at(drawn, &local, deadline)
            } else {
                drawn
            };
            for bag in drawn.bags() {
                let set = rows.set_of(bag.vertices());
                if seen.insert(set.clone()) {
                    pooled.push(set);
                }
            }
            if best
                .as_ref()
                .is_none_or(|held| drawn.quality_key() < held.quality_key())
            {
                best = Some(drawn);
            }
        }
        let Some(mut chosen) = best else {
            return Vec::new();
        };
        if let Some(searched) = search(&pooled, &local, &rows, self.limits, deadline)
            && searched.quality_key() < chosen.quality_key()
        {
            chosen = searched;
        }
        if chosen.treewidth() > width {
            return Vec::new();
        }
        self.cliques_of(&chosen, &order, width, scratch)
    }

    /// The bags of `triangulated`, in the whole graph's numbering, keeping the
    /// ones that are potential maximal cliques of it and no wider than `width`.
    ///
    /// A clique of a triangulation of the local graph is either a potential
    /// maximal clique of the whole graph or a minimal separator of it; the
    /// test drops the separators, which cost the programme a pass over the
    /// graph and split nothing.
    fn cliques_of(
        &self,
        triangulated: &TreeDecomposition,
        order: &[u32],
        width: u32,
        scratch: &mut Scratch,
    ) -> Vec<VertexSet> {
        triangulated
            .bags()
            .iter()
            .map(|bag| {
                let mut vertices: Vec<u32> = bag
                    .vertices()
                    .iter()
                    .map(|&local| order[local as usize])
                    .collect();
                vertices.sort_unstable();
                vertices.dedup();
                vertices
            })
            .filter(|clique| {
                clique.len() as u32 <= width
                    && is_potential_maximal_clique(clique, self.adjacency, scratch)
            })
            .map(|clique| self.adjacency.set_of(&clique))
            .collect()
    }

    /// The local graph on `focus`: what `G` induces there, plus the
    /// neighbourhood of every component of `G − focus` filled into a clique.
    /// Comes back with the vertex list the local numbering reads.
    fn local_graph(&self, focus: &[u32], scratch: &mut Scratch) -> Option<(Graph, Vec<u32>)> {
        let mut order = focus.to_vec();
        order.sort_unstable();
        order.dedup();
        let size = order.len();
        let words = size.div_ceil(64).max(1);
        let set = self.adjacency.set_of(&order);
        for (index, &vertex) in order.iter().enumerate() {
            scratch.position[vertex as usize] = index as u32;
        }
        // The local graph is built as its own rows of words and read out as
        // edges once, so that filling a component's neighbourhood into a
        // clique costs a word per row rather than a pair at a time.
        let mut rows = vec![0u64; words * size];
        for (index, &vertex) in order.iter().enumerate() {
            for (other, &next) in order.iter().enumerate().skip(index + 1) {
                if self.adjacency.adjacent(vertex, next) {
                    rows[index * words + other / 64] |= 1 << (other % 64);
                    rows[other * words + index / 64] |= 1 << (index % 64);
                }
            }
        }
        for border in borders(self.adjacency, &set, scratch) {
            scratch.row.clear();
            scratch.row.resize(words, 0);
            for vertex in border.iter() {
                let local = scratch.position[vertex as usize] as usize;
                scratch.row[local / 64] |= 1 << (local % 64);
            }
            for vertex in border.iter() {
                let local = scratch.position[vertex as usize] as usize;
                let row = &mut rows[local * words..(local + 1) * words];
                for (word, &mask) in row.iter_mut().zip(&scratch.row) {
                    *word |= mask;
                }
                row[local / 64] &= !(1 << (local % 64));
            }
        }
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for left in 0..size {
            for word in 0..words {
                let mut bits = rows[left * words + word];
                while bits != 0 {
                    let right = word * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if right > left {
                        edges.push((left as u32, right as u32));
                    }
                }
            }
        }
        let graph = Graph::new(size as u32, edges);
        Some((graph, order))
    }
}

/// The border of every component of `G − removed`. A component with no border
/// — a part of a disconnected graph that `removed` does not touch — is left
/// out.
fn borders(adjacency: &Adjacency, removed: &VertexSet, scratch: &mut Scratch) -> Vec<VertexSet> {
    adjacency
        .split(removed, scratch)
        .borders
        .into_iter()
        .filter(|border| !border.is_empty())
        .collect()
}

/// Half of what is left, so the level above keeps the other half.
fn share(deadline: Option<Instant>) -> Option<Instant> {
    let deadline = deadline?;
    let left = remaining(deadline);
    Some(deadline - left + left / LEVEL_TIME_SHARE)
}

/// Whether the sorted set `candidate` is a potential maximal clique of the
/// graph — a maximal clique of some minimal triangulation of it.
///
/// The test is the characterisation of Bouchitté and Todinca: no component of
/// `G − K` may have the whole of `K` on its border, and every two non-adjacent
/// vertices of `K` must lie together on the border of some component. The
/// second is the statement that `K` becomes a clique once each component's
/// neighbourhood is filled in, which is what a triangulation does.
pub(super) fn is_potential_maximal_clique(
    candidate: &[u32],
    adjacency: &Adjacency,
    scratch: &mut Scratch,
) -> bool {
    let size = candidate.len();
    if size == 0 {
        return false;
    }
    let set = adjacency.set_of(candidate);
    for (index, &vertex) in candidate.iter().enumerate() {
        scratch.position[vertex as usize] = index as u32;
    }
    let components = borders(adjacency, &set, scratch);
    // The pairs of `K` some component's border holds together, a row of bits
    // per vertex of `K`, in the room the caller keeps for it.
    let words = size.div_ceil(64).max(1);
    let mut together = std::mem::take(&mut scratch.square);
    let mut row = std::mem::take(&mut scratch.row);
    together.clear();
    together.resize(words * size, 0);
    let answer = every_pair_is_together(
        candidate,
        adjacency,
        &components,
        &scratch.position,
        &mut together,
        &mut row,
        words,
    );
    scratch.square = together;
    scratch.row = row;
    answer
}

/// The two halves of the test, over the borders `components`: no border may
/// hold the whole of `candidate`, and every non-adjacent pair of it must lie
/// on one of them.
fn every_pair_is_together(
    candidate: &[u32],
    adjacency: &Adjacency,
    components: &[VertexSet],
    position: &[u32],
    together: &mut [u64],
    row: &mut Vec<u64>,
    words: usize,
) -> bool {
    let size = candidate.len();
    for border in components {
        if border.len() == size {
            // A full component: `K` is contained in one bag of every
            // triangulation that separates on it, so it is not a maximal
            // clique of any of them.
            return false;
        }
        // A component's border is a set of `K`'s own vertices, so the pairs it
        // covers are one row of bits added to each vertex of it.
        row.clear();
        row.resize(words, 0);
        for vertex in border.iter() {
            let local = position[vertex as usize] as usize;
            row[local / 64] |= 1 << (local % 64);
        }
        for vertex in border.iter() {
            let local = position[vertex as usize] as usize;
            for (word, &mask) in together[local * words..(local + 1) * words]
                .iter_mut()
                .zip(row.iter())
            {
                *word |= mask;
            }
        }
    }
    for (left, &one) in candidate.iter().enumerate() {
        for (right, &other) in candidate.iter().enumerate().skip(left + 1) {
            if together[left * words + right / 64] & (1 << (right % 64)) != 0 {
                continue;
            }
            if !adjacency.adjacent(one, other) {
                return false;
            }
        }
    }
    true
}
