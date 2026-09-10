//! Drop the fill edges a decomposition does not need.
//!
//! Completing every bag of a tree decomposition to a clique gives a chordal
//! graph containing the input: a triangulation, whose maximal cliques are the
//! bags of a decomposition of the same width. That triangulation is usually not
//! minimal — some of the edges it added can be taken out again and leave the
//! graph chordal, and the cliques that lose an edge get smaller.
//!
//! Removing one edge `uv` of a chordal graph leaves it chordal exactly when the
//! common neighbourhood of `u` and `v` is a clique, and a triangulation is
//! minimal exactly when no single added edge can be removed (Rose, Tarjan and
//! Lueker, "Algorithmic aspects of vertex elimination on graphs", SIAM Journal
//! on Computing 5(2), 1976). So dropping removable added edges until none is
//! left gives a minimal triangulation, and its clique tree is a decomposition
//! no wider than the one it started from.

use std::time::{Duration, Instant};

use super::TreeDecomposition;
use crate::deadline::{expired, remaining};
use crate::elimination::build_td::build_td_from_ranked_bags;
use crate::elimination::execution::DeadlinePacer;
use crate::elimination::minimal_triangulation::{Reach, cardinality_search};
use crate::{Error, Graph};

#[cfg(test)]
mod tests;

/// A graph as one bitset row per vertex.
struct RowSet {
    rows: Vec<u64>,
    words: usize,
}

impl RowSet {
    fn new(vertices: usize) -> Self {
        let words = vertices.div_ceil(64);
        Self {
            rows: vec![0; vertices * words],
            words,
        }
    }

    fn row(&self, vertex: usize) -> &[u64] {
        &self.rows[vertex * self.words..(vertex + 1) * self.words]
    }

    fn contains(&self, vertex: usize, other: usize) -> bool {
        self.rows[vertex * self.words + other / 64] & (1u64 << (other % 64)) != 0
    }

    fn insert(&mut self, vertex: usize, other: usize) {
        self.rows[vertex * self.words + other / 64] |= 1u64 << (other % 64);
        self.rows[other * self.words + vertex / 64] |= 1u64 << (vertex % 64);
    }

    fn remove(&mut self, vertex: usize, other: usize) {
        self.rows[vertex * self.words + other / 64] &= !(1u64 << (other % 64));
        self.rows[other * self.words + vertex / 64] &= !(1u64 << (vertex % 64));
    }

    /// How many edges the set holds.
    fn edges(&self) -> u64 {
        crate::meter::charge(self.rows.len() as u64);
        self.rows
            .iter()
            .map(|word| u64::from(word.count_ones()))
            .sum::<u64>()
            / 2
    }

    /// The vertices of `row`, in ascending order.
    fn members(row: &[u64], into: &mut Vec<u32>) {
        into.clear();
        for (index, &word) in row.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                into.push((index * 64 + bits.trailing_zeros() as usize) as u32);
                bits &= bits - 1;
            }
        }
    }
}

/// Whether the common neighbourhood of `left` and `right` is a clique, which is
/// what makes the edge between them removable without breaking chordality.
fn common_neighbourhood_is_clique(
    graph: &RowSet,
    left: usize,
    right: usize,
    common: &mut [u64],
) -> bool {
    let mut size = 0u64;
    for (word, (&in_left, &in_right)) in graph
        .row(left)
        .iter()
        .zip(graph.row(right).iter())
        .enumerate()
    {
        common[word] = in_left & in_right;
        size += u64::from((in_left & in_right).count_ones());
    }
    crate::meter::charge(size.saturating_mul(graph.words as u64));
    // Walk the words and bits of `common` rather than listing its vertices
    // first: this test runs once per candidate edge, and the list was the
    // largest single source of writes in the minimalizer.
    for member_word in 0..graph.words {
        let mut bits = common[member_word];
        while bits != 0 {
            let index = member_word * 64 + bits.trailing_zeros() as usize;
            bits &= bits - 1;
            let row = graph.row(index);
            // A vertex is in its own common-neighbourhood word but not in its
            // own row, so that word is compared apart from the rest.
            let own = common[member_word] & !(1u64 << (index % 64));
            if own & !row[member_word] != 0 {
                return false;
            }
            let missing = |(&wanted, &present): (&u64, &u64)| wanted & !present != 0;
            if common[..member_word]
                .iter()
                .zip(&row[..member_word])
                .any(missing)
                || common[member_word + 1..]
                    .iter()
                    .zip(&row[member_word + 1..])
                    .any(missing)
            {
                return false;
            }
        }
    }
    true
}

/// The chordal completion of `decomposition`: every bag made a clique.
///
/// Returns `None` when `deadline` passes before every bag is in. A half-built
/// completion is not a triangulation of anything, so there is nothing to hand
/// back and the caller keeps the decomposition it had.
fn completion(
    decomposition: &TreeDecomposition,
    vertices: usize,
    deadline: Option<Instant>,
) -> Option<RowSet> {
    let mut completion = RowSet::new(vertices);
    let mut pacer = DeadlinePacer::new();
    for bag in decomposition.bags() {
        let bag = bag.vertices();
        // Charged before the bag runs, since one wide bag is millions of
        // inserts and the pacer has to see it coming rather than afterwards.
        crate::meter::charge((bag.len().saturating_mul(bag.len())) as u64);
        if pacer.due() && expired(deadline) {
            return None;
        }
        for (position, &left) in bag.iter().enumerate() {
            for &right in &bag[position + 1..] {
                completion.insert(left as usize, right as usize);
            }
        }
    }
    Some(completion)
}

/// Take fill edges out of `completion` until none is removable, and report how
/// many went.
///
/// How many sweeps that takes is not known in advance, so `deadline` is what
/// bounds the loop. It is read on the pacer's stride, which counts the word
/// scanning each edge test charges, and a sweep cut short leaves the edges it
/// already dropped out: taking a removable edge out of a chordal graph leaves
/// it chordal, so a partly minimalized completion is a triangulation like any
/// other, only with fewer edges gone than a finished run would have.
///
/// A sweep after the first tests only the edges that could have changed
/// answer. Dropping an edge `uv` adds nothing to any common neighbourhood: the
/// only neighbourhoods it changes at all are those of pairs `u` or `v` belongs
/// to, which it shrinks. So an edge whose endpoints have both gone untouched
/// since its last test would fail that test again, and the sweep passes over
/// it. The last sweep, which by construction removes nothing, is the one that
/// gains most from this.
fn minimalize(
    completion: &mut RowSet,
    graph: &Graph,
    vertices: usize,
    deadline: Option<Instant>,
) -> usize {
    let mut original = RowSet::new(vertices);
    crate::meter::charge(graph.edges().len() as u64);
    for &(left, right) in graph.edges() {
        if left != right {
            original.insert(left as usize, right as usize);
        }
    }
    let mut common = vec![0u64; completion.words];
    // The sweep in which a removal last took an edge off each vertex, 0 for a
    // vertex no removal has touched.
    let mut touched: Vec<u32> = vec![0; vertices];
    let mut removed = 0;
    let mut pacer = DeadlinePacer::new();
    let mut pass = 0u32;
    loop {
        pass += 1;
        let mut removed_this_pass = 0;
        for vertex in 0..vertices {
            crate::meter::charge(completion.words as u64);
            if pacer.due() && expired(deadline) {
                return removed + removed_this_pass;
            }
            let words = completion.words;
            let base = vertex * words;
            let first = vertex / 64;
            for word in first..words {
                let mut bits = completion.rows[base + word];
                if word == first {
                    // `vertex` itself and everything below it: those pairs are
                    // tested from their smaller endpoint instead.
                    bits &= !1u64 << (vertex % 64);
                }
                // A removal below clears bits of this row, but only the bit of
                // the member being tested, which this walk has already taken
                // out of `bits`.
                while bits != 0 {
                    let other = word * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if original.contains(vertex, other) {
                        continue;
                    }
                    if touched[vertex] + 2 <= pass && touched[other] + 2 <= pass {
                        continue;
                    }
                    if pacer.due() && expired(deadline) {
                        return removed + removed_this_pass;
                    }
                    if common_neighbourhood_is_clique(completion, vertex, other, &mut common) {
                        completion.remove(vertex, other);
                        touched[vertex] = pass;
                        touched[other] = pass;
                        removed_this_pass += 1;
                    }
                }
            }
        }
        removed += removed_this_pass;
        if removed_this_pass == 0 {
            return removed;
        }
    }
}

/// The decomposition whose bags are the cliques a perfect elimination ordering
/// of `completion` produces.
fn decompose_completion(
    completion: &RowSet,
    vertices: usize,
    deadline: Option<Instant>,
) -> Option<TreeDecomposition> {
    let mut adjacency: Vec<Vec<u32>> = Vec::with_capacity(vertices);
    let mut members: Vec<u32> = Vec::new();
    let mut pacer = DeadlinePacer::new();
    for vertex in 0..vertices {
        crate::meter::charge(completion.words as u64);
        if pacer.due() && expired(deadline) {
            return None;
        }
        RowSet::members(completion.row(vertex), &mut members);
        adjacency.push(members.clone());
    }
    let selected = cardinality_search(&adjacency, Reach::Neighbours, deadline)?;
    let mut rank = vec![0u32; vertices];
    for (step, &vertex) in selected.iter().rev().enumerate() {
        rank[vertex as usize] = step as u32;
    }
    let mut bags: Vec<Vec<u32>> = Vec::with_capacity(vertices);
    for &vertex in selected.iter().rev() {
        crate::meter::charge(adjacency[vertex as usize].len() as u64);
        if pacer.due() && expired(deadline) {
            return None;
        }
        let step = rank[vertex as usize];
        let mut bag = vec![vertex];
        bag.extend(
            adjacency[vertex as usize]
                .iter()
                .copied()
                .filter(|&neighbour| rank[neighbour as usize] > step),
        );
        bags.push(bag);
    }
    Some(build_td_from_ranked_bags(bags, &rank))
}

/// Rebuild `decomposition` on a minimal triangulation of `graph`.
///
/// Every bag of `decomposition` is completed to a clique, the added edges that
/// can go without breaking chordality are dropped until none is left, and the
/// cliques of what remains become the new bags. The result is never wider than
/// `decomposition`, and never worse on `(width, total bag size)`: where the
/// rebuilt decomposition does not improve on that pair, the input comes back
/// unchanged.
///
/// `budget` bounds the pass, and is what a caller with a deadline of its own
/// should hand over rather than a size limit. The pass costs about what
/// completing the bags costs, which the decomposition says in advance, so a
/// budget that cannot cover that much declines the pass and returns
/// `decomposition` untouched. Past that point every loop reads the clock on a
/// stride: the completion and the rebuild return `decomposition` unchanged when
/// they run out of time, and the edge-dropping sweeps in between keep the edges
/// they had already dropped. So a run out of budget returns a decomposition
/// either way, and never later than the budget.
///
/// The pass holds two bitsets over the graph's vertices, so its memory grows
/// with the square of the vertex count. A caller running it under a deadline
/// should keep that in mind on a large graph.
///
/// # Errors
///
/// Returns an error if `decomposition` is not a valid decomposition of `graph`,
/// or if the budget is too large to represent as a deadline.
pub fn minimalize_triangulation(
    decomposition: TreeDecomposition,
    graph: &Graph,
    budget: Option<Duration>,
) -> Result<TreeDecomposition, Error> {
    decomposition.validate(graph)?;
    let deadline = budget
        .map(|budget| crate::deadline::checked(crate::meter::now(), budget, "minimalization"))
        .transpose()?;
    Ok(minimalize_at(decomposition, graph, deadline))
}

/// What the pass costs before it can drop anything, in the units the loops
/// charge: completing the bags is one insert per pair of a bag, and the rebuild
/// after it scans every vertex once per step of its search and walks every edge
/// of the completion once. A completion has at most as many edges as there are
/// pairs in the bags, so twice the bag squares covers both.
fn projected_units(decomposition: &TreeDecomposition, vertices: usize) -> u64 {
    let squares = decomposition
        .bags()
        .iter()
        .map(|bag| {
            let size = bag.vertices().len() as u64;
            size.saturating_mul(size)
        })
        .fold(0u64, u64::saturating_add);
    let vertices = vertices as u64;
    squares
        .saturating_mul(2)
        .saturating_add(vertices.saturating_mul(vertices))
}

/// Whether there is time to run the pass over `decomposition` before `deadline`.
///
/// The size of a graph does not say what the pass costs; the size of the bags
/// behind it does, and by the time a caller asks, it has them. So the rule is
/// the projection against the clock, the way the trailing FlowCutter candidate
/// asks whether its first restart fits before it starts one. A caller that
/// gates on size first still wants this: the gate keeps the memory bounded, and
/// this keeps a graph from spending a window it does not have.
pub(crate) fn minimalize_fits(
    decomposition: &TreeDecomposition,
    graph: &Graph,
    deadline: Option<Instant>,
) -> bool {
    let Some(deadline) = deadline else {
        return true;
    };
    let vertices = graph.num_vertices() as usize;
    let projected = projected_units(decomposition, vertices);
    Duration::from_millis(crate::meter::milliseconds_for_units(projected)) < remaining(deadline)
}

/// [`minimalize_triangulation`] against an absolute deadline, for a caller that
/// already holds one. The decomposition comes back unchanged when the pass
/// does not fit in what is left of the deadline, so a caller does not check
/// [`minimalize_fits`] itself.
pub(crate) fn minimalize_at(
    decomposition: TreeDecomposition,
    graph: &Graph,
    deadline: Option<Instant>,
) -> TreeDecomposition {
    let vertices = graph.num_vertices() as usize;
    if vertices == 0 || !minimalize_fits(&decomposition, graph, deadline) {
        return decomposition;
    }
    let Some(mut completion) = completion(&decomposition, vertices, deadline) else {
        return decomposition;
    };
    // The sweeps stop early enough to leave the rebuild its own time. Without
    // that they would run to the deadline itself and the rebuild would put the
    // whole pass past it, which is the one outcome a caller cannot use: it has
    // a decomposition either way, and only the clock decides whether anyone is
    // still waiting for it.
    // A millisecond on top of the estimate, because the estimate rounds down to
    // whole milliseconds and a small graph would otherwise reserve nothing.
    let rebuild = Duration::from_millis(
        crate::meter::milliseconds_for_units(rebuild_units(vertices as u64, completion.edges()))
            .saturating_add(1),
    );
    let sweep_deadline = deadline.map(|deadline| deadline.checked_sub(rebuild).unwrap_or(deadline));
    if minimalize(&mut completion, graph, vertices, sweep_deadline) == 0 {
        return decomposition;
    }
    let Some(rebuilt) = decompose_completion(&completion, vertices, deadline) else {
        return decomposition;
    };
    if rebuilt.quality_key() < decomposition.quality_key() {
        rebuilt
    } else {
        decomposition
    }
}

/// What rebuilding the bags from a completion of `vertices` vertices and
/// `edges` edges costs, in the units the loops charge: the adjacency copy and
/// the search each walk every edge, and the search scans every vertex once per
/// step.
fn rebuild_units(vertices: u64, edges: u64) -> u64 {
    vertices
        .saturating_mul(vertices)
        .saturating_add(edges.saturating_mul(4))
}
