//! Re-triangulate between two of the decompositions a run already built.
//!
//! The recombination stage of [`super`] pools the bags of every tree the run
//! produced and searches that pool; the merge loop of [`super::merge`] builds a
//! fresh tree, improves it on its own and merges it in. Neither adds the
//! cliques of a *local re-triangulation* between a bag of one tree and a bag of
//! another, which is the step of Tamaki, "Heuristic computation of exact
//! treewidth", 2022, section 3, and where that paper says the tree decompositions
//! neither list admits on its own come from.
//!
//! The step, for a bag `X` of the answer and a bag `Y` of another pooled tree:
//! `C` is the largest component of `G − X`, and `Y` has to lie inside `N[C]`
//! and hold no more vertices than the current width — the first so the two bags
//! do not cross, the second so a tree built through them can be narrower than
//! the one there is. `D` is then the component of `G − Y` whose closed
//! neighbourhood holds `X`, and `U = N[C] ∩ N[D]` is the piece the two bags
//! leave between them. The piece is triangulated on its own, and where it comes
//! back no wider than the current width its cliques go into the pool. Small
//! pieces are worth several draws and a programme of their own, which is what
//! the paper solves exactly. The programme is then run again over the longer
//! list.
//!
//! What is new here against the merge loop is where the partner comes from:
//! the trees of the run's own portfolio, which cut the graph in places a
//! randomised elimination does not, rather than one more randomised
//! elimination. The bags `X` are the answer's own widest, since a re-triangulation
//! anywhere else cannot lower the width.

use std::time::Instant;

use rustc_hash::FxHashSet;

use super::merge::{Loop, bags_of};
use super::sets::{Adjacency, Scratch, VertexSet};
use super::{BagPool, search};
use crate::deadline::expired;
use crate::{Graph, TreeDecomposition};

/// Pieces small enough for several draws and a programme of their own. The
/// paper's `BASE_SIZE`, which is where it switches from a greedy triangulation
/// to an exact one.
const BASE_SIZE: usize = 60;
/// Draws taken of a small piece.
const SMALL_PIECE_DRAWS: usize = 8;
/// Bags of the answer paired with each pooled tree, widest first.
const ANCHORS_PER_TREE: usize = 16;
/// Passes over the pooled trees.
const ROUNDS: usize = 2;

/// Add the cliques of the local re-triangulations between `start` and the other
/// trees in `pool`, and search the longer list.
///
/// Returns a decomposition only where it is narrower than `start`; `None` where
/// the pool is empty, the deadline passes, or the search would exceed its
/// [`super::Limits`].
pub(crate) fn local_merge(
    pool: &BagPool,
    graph: &Graph,
    start: &TreeDecomposition,
    seed: u64,
    deadline: Option<Instant>,
) -> Option<TreeDecomposition> {
    if pool.is_empty() || expired(deadline) {
        return None;
    }
    let adjacency = Adjacency::of(graph)?;
    let sources = pool.assemble_by_source(graph, &adjacency, deadline);
    if sources.is_empty() {
        return None;
    }
    let limits = pool.limits;
    let mut state = Loop::new(graph, &adjacency, limits, seed);
    let mut scratch = Scratch::new(&adjacency);

    // The list begins with the answer's own bags, so the programme over it
    // cannot come back wider than the answer, and the pool's on top of them.
    let mut list = List::new(bags_of(start, &adjacency), limits);
    for source in &sources {
        for bag in source {
            list.add(bag.clone());
        }
    }
    let mut best = start.clone();
    for _ in 0..ROUNDS {
        let mut round_moved = false;
        for partners in &sources {
            if expired(deadline) {
                break;
            }
            let width = best.treewidth();
            let mut fresh = false;
            for anchor in anchors(&best, &adjacency) {
                if expired(deadline) {
                    break;
                }
                let focuses =
                    state.focuses_from(&anchor, partners, width as usize, &mut scratch, deadline);
                for focus in focuses {
                    if expired(deadline) || list.full() {
                        break;
                    }
                    let draws = if focus.len() <= BASE_SIZE {
                        SMALL_PIECE_DRAWS
                    } else {
                        1
                    };
                    let cliques =
                        state.triangulate_pooled(&focus, width, draws, &mut scratch, deadline);
                    for clique in cliques {
                        fresh |= list.add(clique);
                    }
                }
            }
            if !fresh || expired(deadline) {
                continue;
            }
            let Some(built) = search(&list.bags, graph, &adjacency, limits, deadline) else {
                break;
            };
            if built.quality_key() < best.quality_key() {
                best = built;
                round_moved = true;
                // At width k no tree of width k or less has a bag of more than
                // k + 2 vertices, so the list drops them: they are what the
                // merges would otherwise pile up.
                list.trim(best.treewidth() as usize + 2);
            }
        }
        if !round_moved {
            break;
        }
    }
    (best.quality_key() < start.quality_key()).then_some(best)
}

/// The bags of `answer` to re-triangulate around, widest first.
fn anchors(answer: &TreeDecomposition, adjacency: &Adjacency) -> Vec<VertexSet> {
    let mut order: Vec<usize> = (0..answer.bags().len()).collect();
    order.sort_by_key(|&index| std::cmp::Reverse(answer.bags()[index].vertices().len()));
    order.truncate(ANCHORS_PER_TREE);
    order
        .into_iter()
        .map(|index| adjacency.set_of(answer.bags()[index].vertices()))
        .collect()
}

/// The list the programme reads, under the pool's caps and without repeats.
struct List {
    bags: Vec<VertexSet>,
    held: FxHashSet<VertexSet>,
    stored: usize,
    limits: super::Limits,
}

impl List {
    fn new(bags: Vec<VertexSet>, limits: super::Limits) -> Self {
        let held: FxHashSet<VertexSet> = bags.iter().cloned().collect();
        let stored = bags.iter().map(VertexSet::len).sum();
        Self {
            bags,
            held,
            stored,
            limits,
        }
    }

    /// Whether either cap is reached.
    fn full(&self) -> bool {
        self.bags.len() >= self.limits.bags || self.stored >= self.limits.pool_vertices
    }

    /// Take `bag` unless it is already there or a cap is reached. Returns
    /// whether the list grew.
    fn add(&mut self, bag: VertexSet) -> bool {
        if self.full() || !self.held.insert(bag.clone()) {
            return false;
        }
        self.stored += bag.len();
        self.bags.push(bag);
        true
    }

    /// Drop every bag of more than `room` vertices.
    fn trim(&mut self, room: usize) {
        self.bags.retain(|bag| bag.len() <= room);
        self.held = self.bags.iter().cloned().collect();
        self.stored = self.bags.iter().map(VertexSet::len).sum();
    }
}
