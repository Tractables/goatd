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

use rustc_hash::{FxHashMap, FxHashSet};

use super::{Limits, Neighbourhoods, adjacency_lists, search};
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
    bags: Vec<Vec<u32>>,
    tree: TreeDecomposition,
}

impl Answer {
    fn width(&self) -> u32 {
        self.tree.treewidth()
    }
}

/// What every level of the recursion shares.
struct Loop<'a> {
    graph: &'a Graph,
    adjacency: &'a [Vec<u32>],
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
    let adjacency = adjacency_lists(graph);
    let mut state = Loop {
        graph,
        adjacency: &adjacency,
        limits,
        rng: Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET)),
        weights: vec![0; graph.num_vertices() as usize],
    };
    let mut answer = match start {
        Some(start) => {
            let minimalised = if crate::decomposition::minimalize_fits(start, graph, deadline) {
                crate::decomposition::minimalize_at(start.clone(), graph, deadline)
            } else {
                start.clone()
            };
            state.settle(bags_of(&minimalised), deadline)?
        }
        None => state.initial(deadline)?,
    };
    let mut neighbourhoods = Neighbourhoods::new(&adjacency);
    // Every round keeps what it merged in, whether or not the width moved. The
    // list only grows, so the programme over it never reads a wider tree than
    // the round before, and one merge on its own rarely lowers anything: what
    // lowers the width is the bags of several of them together. The round that
    // adds nothing at all is the one worth stopping on.
    while !expired(deadline) {
        let held = answer.bags.len();
        let Some(next) = state.improve(&answer, 0, &mut neighbourhoods, deadline) else {
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
fn bags_of(decomposition: &TreeDecomposition) -> Vec<Vec<u32>> {
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
    bags
}

impl Loop<'_> {
    /// Run the programme over `bags` and keep both.
    fn settle(&self, bags: Vec<Vec<u32>>, deadline: Option<Instant>) -> Option<Answer> {
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
            let drawn = if crate::decomposition::minimalize_fits(&drawn, self.graph, until) {
                crate::decomposition::minimalize_at(drawn, self.graph, until)
            } else {
                drawn
            };
            if best
                .as_ref()
                .is_none_or(|best| drawn.quality_key() < best.quality_key())
            {
                best = Some(drawn);
            }
        }
        self.settle(bags_of(&best?), deadline)
    }

    /// One improvement of `answer`: build a side answer, improve it until it is
    /// no wider, merge it in and run the programme over the merged list.
    fn improve(
        &mut self,
        answer: &Answer,
        depth: usize,
        neighbourhoods: &mut Neighbourhoods<'_>,
        deadline: Option<Instant>,
    ) -> Option<Answer> {
        let until = share(deadline);
        let mut side = self.initial(until)?;
        while side.width() > answer.width() && depth + 1 < MAX_DEPTH && !expired(until) {
            let held = side.bags.len();
            let Some(better) = self.improve(&side, depth + 1, neighbourhoods, until) else {
                break;
            };
            side = better;
            if side.bags.len() <= held {
                break;
            }
        }
        let merged = self.merge(answer, &side, neighbourhoods, deadline);
        self.settle(merged, deadline)
    }

    /// The merged list: both lists, and the cliques of a minimal triangulation
    /// of each focus the two answers pick out between them.
    fn merge(
        &mut self,
        answer: &Answer,
        side: &Answer,
        neighbourhoods: &mut Neighbourhoods<'_>,
        deadline: Option<Instant>,
    ) -> Vec<Vec<u32>> {
        let width = answer.width();
        let mut bags = answer.bags.clone();
        let mut held: FxHashSet<Vec<u32>> = bags.iter().cloned().collect();
        for bag in &side.bags {
            if held.insert(bag.clone()) {
                bags.push(bag.clone());
            }
        }
        let Some(focuses) = self.focuses(answer, side, neighbourhoods, deadline) else {
            return bags;
        };
        let mut stored: usize = bags.iter().map(Vec::len).sum();
        for focus in focuses {
            if expired(deadline) || bags.len() >= self.limits.bags {
                break;
            }
            for clique in self.triangulate(&focus, width, neighbourhoods, deadline) {
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
        neighbourhoods: &mut Neighbourhoods<'_>,
        deadline: Option<Instant>,
    ) -> Option<Vec<Vec<u32>>> {
        let width = answer.width() as usize;
        let pick = (self.rng.next_u64() % answer.bags.len() as u64) as usize;
        let chosen = &answer.bags[pick];
        let split = neighbourhoods.split(chosen);
        let largest = split
            .components
            .iter()
            .zip(&split.separators)
            .max_by_key(|(component, _)| component.len())?;
        let closed = closed_neighbourhood(largest.0, largest.1);
        let inside: FxHashSet<u32> = closed.iter().copied().collect();
        let mut found: Vec<Vec<u32>> = Vec::new();
        for (index, partner) in side.bags.iter().enumerate() {
            if index % DEADLINE_STRIDE == 0 && expired(deadline) {
                break;
            }
            if partner.len() > width || !partner.iter().all(|vertex| inside.contains(vertex)) {
                continue;
            }
            let across = neighbourhoods.split(partner);
            let Some((component, border)) = across
                .components
                .iter()
                .zip(&across.separators)
                .find(|(component, border)| holds(component, border, chosen))
            else {
                continue;
            };
            let far = closed_neighbourhood(component, border);
            let focus: Vec<u32> = far
                .into_iter()
                .filter(|vertex| inside.contains(vertex))
                .collect();
            if focus.len() > 1 {
                found.push(focus);
            }
        }
        found.sort_by(|one, other| one.len().cmp(&other.len()).then_with(|| one.cmp(other)));
        found.dedup();
        found.truncate(FOCUSES_PER_MERGE);
        Some(found)
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
        neighbourhoods: &mut Neighbourhoods<'_>,
        deadline: Option<Instant>,
    ) -> Vec<Vec<u32>> {
        let Some((local, order)) = self.local_graph(focus, neighbourhoods) else {
            return Vec::new();
        };
        let budget = deadline.map(|deadline| remaining(deadline) / LEVEL_TIME_SHARE);
        let Ok(triangulated) = crate::elimination::decompose(
            &local,
            crate::elimination::Order::MinimalTriangulation,
            0,
            budget,
        ) else {
            return Vec::new();
        };
        if triangulated.treewidth() > width {
            return Vec::new();
        }
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
                clique.len() as u32 <= width && is_potential_maximal_clique(clique, neighbourhoods)
            })
            .collect()
    }

    /// The local graph on `focus`: what `G` induces there, plus the
    /// neighbourhood of every component of `G − focus` filled into a clique.
    /// Comes back with the vertex list the local numbering reads.
    fn local_graph(
        &self,
        focus: &[u32],
        neighbourhoods: &mut Neighbourhoods<'_>,
    ) -> Option<(Graph, Vec<u32>)> {
        let mut order = focus.to_vec();
        order.sort_unstable();
        order.dedup();
        let position = |vertex: u32| order.binary_search(&vertex).ok().map(|index| index as u32);
        let mut edges: FxHashSet<(u32, u32)> = FxHashSet::default();
        for (index, &vertex) in order.iter().enumerate() {
            for &next in &self.adjacency[vertex as usize] {
                if let Some(other) = position(next)
                    && (other as usize) > index
                {
                    edges.insert((index as u32, other));
                }
            }
        }
        for border in neighbourhoods.borders(&order) {
            for (index, &left) in border.iter().enumerate() {
                let Some(left) = position(left) else {
                    continue;
                };
                for &right in &border[index + 1..] {
                    let Some(right) = position(right) else {
                        continue;
                    };
                    edges.insert((left.min(right), left.max(right)));
                }
            }
        }
        let edges: Vec<(u32, u32)> = edges.into_iter().collect();
        let graph = Graph::new(order.len() as u32, edges);
        Some((graph, order))
    }
}

/// A component's closed neighbourhood, sorted.
fn closed_neighbourhood(vertices: &[u32], border: &[u32]) -> Vec<u32> {
    let mut closed = vertices.to_vec();
    closed.extend_from_slice(border);
    closed.sort_unstable();
    closed.dedup();
    closed
}

/// Whether every vertex of `set` is in the component or on its border.
fn holds(component: &[u32], border: &[u32], set: &[u32]) -> bool {
    let inside: FxHashSet<u32> = component.iter().chain(border).copied().collect();
    set.iter().all(|vertex| inside.contains(vertex))
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
    neighbourhoods: &mut Neighbourhoods<'_>,
) -> bool {
    let size = candidate.len();
    if size == 0 {
        return false;
    }
    let mut position: FxHashMap<u32, usize> = FxHashMap::default();
    for (index, &vertex) in candidate.iter().enumerate() {
        position.insert(vertex, index);
    }
    let mut together = vec![false; size * size];
    for border in neighbourhoods.borders(candidate) {
        if border.len() == size {
            // A full component: `K` is contained in one bag of every
            // triangulation that separates on it, so it is not a maximal
            // clique of any of them.
            return false;
        }
        for (index, &left) in border.iter().enumerate() {
            let Some(&left) = position.get(&left) else {
                continue;
            };
            for &right in &border[index + 1..] {
                let Some(&right) = position.get(&right) else {
                    continue;
                };
                together[left * size + right] = true;
                together[right * size + left] = true;
            }
        }
    }
    for (left, &one) in candidate.iter().enumerate() {
        for (right, &other) in candidate.iter().enumerate().skip(left + 1) {
            if together[left * size + right] {
                continue;
            }
            if !neighbourhoods.adjacent(one, other) {
                return false;
            }
        }
    }
    true
}
