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

pub mod vertex_rebuild;

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

    /// This set restricted to the vertices other than `vertex`, the ids above
    /// it lowered by one, over `vertices - 1` vertices: what completing the
    /// bags of a decomposition projected off `vertex` gives, since a pair
    /// shares a bag of the projection exactly when it shared one before.
    fn without_vertex(&self, vertex: usize) -> RowSet {
        let vertices = self.rows.len() / self.words.max(1);
        let mut out = RowSet::new(vertices.saturating_sub(1));
        for (row, target) in (0..vertices)
            .filter(|&row| row != vertex)
            .zip(out.rows.chunks_mut(out.words.max(1)))
        {
            let row = self.row(row);
            for (index, word) in target.iter_mut().enumerate() {
                *word = word_without(row, index, vertex);
            }
        }
        out
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

/// Word `index` of `row` as it reads with `vertex` taken out and the ids
/// above it lowered by one: bits above `vertex` move down one place, and the
/// bit that enters at the top comes from the next word.
fn word_without(row: &[u64], index: usize, vertex: usize) -> u64 {
    let source = |index: usize| row.get(index).copied().unwrap_or(0);
    let word_v = vertex / 64;
    if index < word_v {
        return source(index);
    }
    let here = source(index);
    let shifted = (here >> 1) | (source(index + 1) << 63);
    if index == word_v {
        let below = (1u64 << (vertex % 64)) - 1;
        (here & below) | (shifted & !below)
    } else {
        shifted
    }
}

/// Whether the common neighbourhood of `left` and `right` is a clique, which is
/// what makes the edge between them removable without breaking chordality:
/// the size of that neighbourhood, and two of its members that are not
/// adjacent when it is not a clique.
///
/// `hint` is such a pair from an earlier test of the same edge: when both
/// are still in the neighbourhood and still not adjacent the answer is known
/// without walking the members. The test reads the same rows either way and
/// charges the same; only the order it looks in changes, and every order
/// gives the same answer.
fn common_neighbourhood_is_clique(
    graph: &RowSet,
    left: usize,
    right: usize,
    common: &mut [u64],
    hint: Option<(usize, usize)>,
) -> (u64, Option<(usize, usize)>) {
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
    if let Some((a, b)) = hint
        && common[a / 64] & (1u64 << (a % 64)) != 0
        && common[b / 64] & (1u64 << (b % 64)) != 0
        && !graph.contains(a, b)
    {
        return (size, Some((a, b)));
    }
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
            let absent = own & !row[member_word];
            if absent != 0 {
                return (
                    size,
                    Some((index, member_word * 64 + absent.trailing_zeros() as usize)),
                );
            }
            let missing = |(word, (&wanted, &present)): (usize, (&u64, &u64))| {
                let absent = wanted & !present;
                (absent != 0).then(|| word * 64 + absent.trailing_zeros() as usize)
            };
            if let Some(other) = common[..member_word]
                .iter()
                .zip(&row[..member_word])
                .enumerate()
                .find_map(missing)
                .or_else(|| {
                    common[member_word + 1..]
                        .iter()
                        .zip(&row[member_word + 1..])
                        .enumerate()
                        .find_map(|(word, pair)| missing((member_word + 1 + word, pair)))
                })
            {
                return (size, Some((index, other)));
            }
        }
    }
    (size, None)
}

/// What a sweep knows about an edge before testing it.
enum Known {
    Nothing,
    /// A pair that failed an earlier test of the edge, worth looking at first.
    Hint((usize, usize)),
    /// The test fails, and its common neighbourhood has this many members:
    /// nothing that could have changed the answer has changed.
    Fails(u64),
}

/// Where a sweep keeps what its failed clique tests found, so that later
/// tests of the same edge can start from it.
trait Witnesses {
    /// The sweep is about to test the edges out of `vertex`, in ascending
    /// order of the other endpoint.
    fn start_row(&mut self, vertex: usize);
    /// What is known about the edge `vertex`–`other`. `untouched` says that
    /// no removal of this sweep has touched either endpoint, so its
    /// neighbourhoods are as they were when the sweep began.
    fn lookup(&mut self, vertex: usize, other: usize, untouched: bool) -> Known;
    /// The edge last looked up failed on `pair`, with `size` common
    /// neighbours; neither endpoint nor either of the pair has been touched
    /// by a removal of this sweep.
    fn record(&mut self, size: u64, pair: (usize, usize));
}

/// The plain pass keeps nothing: it tests each edge once per sweep.
struct NoWitnesses;

impl Witnesses for NoWitnesses {
    fn start_row(&mut self, _vertex: usize) {}
    fn lookup(&mut self, _vertex: usize, _other: usize, _untouched: bool) -> Known {
        Known::Nothing
    }
    fn record(&mut self, _size: u64, _pair: (usize, usize)) {}
}

const NO_PAIR: (u32, u32) = (u32::MAX, u32::MAX);

/// One failed clique test per fill edge of a round's completion, in the
/// graph's own ids, shared by the rebuilds of the round.
///
/// Each rebuild sweeps that completion less one vertex. While a sweep has
/// removed nothing at either endpoint of a fill edge, the edge's common
/// neighbourhood is the round's less the dropped vertex, and a pair of its
/// members that the round's completion does not join is still not joined:
/// the test fails, on that pair, with one member fewer when the dropped
/// vertex was one. So one full test of the edge, in whichever rebuild comes
/// first, answers it for every later rebuild of the round that drops
/// neither vertex of the pair, and the sweep is charged as if it had read
/// the rows.
#[derive(Default)]
struct FillWitnesses {
    /// Row starts into the three lists, one per vertex and a sentinel.
    starts: Vec<usize>,
    /// The higher endpoint of each fill edge, by lower endpoint, ascending.
    others: Vec<u32>,
    /// Members of the edge's common neighbourhood in the round's completion,
    /// `u32::MAX` until a test has counted them.
    sizes: Vec<u32>,
    pairs: Vec<(u32, u32)>,
}

impl FillWitnesses {
    /// Start over for the fill edges of `completion` over `original`.
    fn rebuild(&mut self, completion: &RowSet, original: &RowSet) {
        let vertices = completion.rows.len() / completion.words.max(1);
        self.starts.clear();
        self.others.clear();
        for vertex in 0..vertices {
            self.starts.push(self.others.len());
            let first = vertex / 64;
            let fill = completion.row(vertex).iter().zip(original.row(vertex));
            for (word, (&filled, &given)) in fill.enumerate().skip(first) {
                let mut bits = filled & !given;
                if word == first {
                    bits &= (!1u64) << (vertex % 64);
                }
                while bits != 0 {
                    self.others
                        .push((word * 64 + bits.trailing_zeros() as usize) as u32);
                    bits &= bits - 1;
                }
            }
        }
        self.starts.push(self.others.len());
        self.sizes.clear();
        self.sizes.resize(self.others.len(), u32::MAX);
        self.pairs.clear();
        self.pairs.resize(self.others.len(), NO_PAIR);
    }
}

/// [`FillWitnesses`] read from a sweep over `completion` less `vertex`,
/// whose ids above it sit one lower.
struct RebuildWitnesses<'a> {
    store: &'a mut FillWitnesses,
    /// The dropped vertex's row of the round's completion.
    dropped: &'a [u64],
    vertex: usize,
    /// The row being swept, in the graph's ids, and the cursor over its
    /// entries.
    left: usize,
    cursor: usize,
    end: usize,
    /// The store entry of the edge last looked up, when it has one.
    slot: Option<usize>,
}

impl RebuildWitnesses<'_> {
    fn raise(&self, v: usize) -> usize {
        v + usize::from(v >= self.vertex)
    }

    /// Whether the dropped vertex is a common neighbour of `left` and
    /// `right` in the round's completion, in the graph's ids.
    fn drops_member(&self, left: usize, right: usize) -> u64 {
        (self.dropped[left / 64] >> (left % 64)) & (self.dropped[right / 64] >> (right % 64)) & 1
    }
}

impl Witnesses for RebuildWitnesses<'_> {
    fn start_row(&mut self, vertex: usize) {
        self.left = self.raise(vertex);
        self.cursor = self.store.starts[self.left];
        self.end = self.store.starts[self.left + 1];
        self.slot = None;
    }

    fn lookup(&mut self, vertex: usize, other: usize, untouched: bool) -> Known {
        let (left, right) = (self.raise(vertex), self.raise(other));
        while self.cursor < self.end && (self.store.others[self.cursor] as usize) < right {
            self.cursor += 1;
        }
        self.slot = None;
        if self.cursor >= self.end || self.store.others[self.cursor] as usize != right {
            return Known::Nothing;
        }
        self.slot = Some(self.cursor);
        let (a, b) = self.store.pairs[self.cursor];
        let (a, b) = (a as usize, b as usize);
        if (a as u32, b as u32) == NO_PAIR || a == self.vertex || b == self.vertex {
            return Known::Nothing;
        }
        let size = self.store.sizes[self.cursor];
        if untouched && size != u32::MAX {
            return Known::Fails(u64::from(size) - self.drops_member(left, right));
        }
        let lower = |v: usize| v - usize::from(v > self.vertex);
        Known::Hint((lower(a), lower(b)))
    }

    fn record(&mut self, size: u64, (a, b): (usize, usize)) {
        if let Some(slot) = self.slot {
            let right = self.store.others[slot] as usize;
            let size = size + self.drops_member(self.left, right);
            self.store.sizes[slot] = u32::try_from(size).unwrap_or(u32::MAX);
            self.store.pairs[slot] = (self.raise(a) as u32, self.raise(b) as u32);
        }
    }
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
///
/// `original_word` gives a word of a row of the graph's own edges, which the
/// sweep never drops; `edge_count` is how many such edges there are, the
/// work reading them is charged as.
fn minimalize(
    completion: &mut RowSet,
    vertices: usize,
    edge_count: usize,
    original_word: impl Fn(usize, usize) -> u64,
    witnesses: &mut impl Witnesses,
    deadline: Option<Instant>,
) -> usize {
    crate::meter::charge(edge_count as u64);
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
            witnesses.start_row(vertex);
            for word in first..words {
                let mut bits = completion.rows[base + word] & !original_word(vertex, word);
                if word == first {
                    // `vertex` itself and everything below it: those pairs are
                    // tested from their smaller endpoint instead.
                    bits &= (!1u64) << (vertex % 64);
                }
                // A removal below clears bits of this row, but only the bit of
                // the member being tested, which this walk has already taken
                // out of `bits`.
                while bits != 0 {
                    let other = word * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    // An endpoint touched in pass p is retested in pass p and
                    // in pass p + 1: an edge tested earlier in pass p saw the
                    // neighbourhood as it was before that removal.
                    if touched[vertex] + 2 <= pass && touched[other] + 2 <= pass {
                        continue;
                    }
                    if pacer.due() && expired(deadline) {
                        return removed + removed_this_pass;
                    }
                    let untouched = touched[vertex] == 0 && touched[other] == 0;
                    let hint = match witnesses.lookup(vertex, other, untouched) {
                        Known::Fails(size) => {
                            // What the test would have charged for its rows.
                            crate::meter::charge(size.saturating_mul(words as u64));
                            continue;
                        }
                        Known::Hint(pair) => Some(pair),
                        Known::Nothing => None,
                    };
                    match common_neighbourhood_is_clique(
                        completion,
                        vertex,
                        other,
                        &mut common,
                        hint,
                    ) {
                        (_, None) => {
                            completion.remove(vertex, other);
                            touched[vertex] = pass;
                            touched[other] = pass;
                            removed_this_pass += 1;
                        }
                        (size, Some((a, b))) => {
                            if untouched && touched[a] == 0 && touched[b] == 0 {
                                witnesses.record(size, (a, b));
                            }
                        }
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
    fits(decomposition, graph.num_vertices() as usize, deadline)
}

fn fits(decomposition: &TreeDecomposition, vertices: usize, deadline: Option<Instant>) -> bool {
    let Some(deadline) = deadline else {
        return true;
    };
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
    match minimalization_candidate(&decomposition, graph, deadline) {
        Some(rebuilt) if rebuilt.quality_key() < decomposition.quality_key() => rebuilt,
        _ => decomposition,
    }
}

/// Build the refined triangulation, leaving the caller's selection policy out.
fn minimalization_candidate(
    decomposition: &TreeDecomposition,
    graph: &Graph,
    deadline: Option<Instant>,
) -> Option<TreeDecomposition> {
    let vertices = graph.num_vertices() as usize;
    if vertices == 0 || !fits(decomposition, vertices, deadline) {
        return None;
    }
    let completion = completion(decomposition, vertices, deadline)?;
    let original = original_edges(graph);
    refine_completion(
        completion,
        vertices,
        graph.edges().len(),
        |row, word| original.row(row)[word],
        &mut NoWitnesses,
        deadline,
    )
}

/// The graph's own edges as a row set.
fn original_edges(graph: &Graph) -> RowSet {
    let mut original = RowSet::new(graph.num_vertices() as usize);
    for &(left, right) in graph.edges() {
        if left != right {
            original.insert(left as usize, right as usize);
        }
    }
    original
}

/// What the vertex reinsertion pass shares across its rebuilds: the edges of
/// the graph, and the completion of the tree the rebuilds start from, both
/// in the graph's own ids. Each rebuild derives its completion from the
/// second by dropping one vertex instead of completing its bags again.
pub(super) struct SharedCompletion {
    original: RowSet,
    completion: RowSet,
    witnesses: FillWitnesses,
}

impl SharedCompletion {
    pub(super) fn new(graph: &Graph) -> Self {
        let vertices = graph.num_vertices() as usize;
        Self {
            original: original_edges(graph),
            completion: RowSet::new(vertices),
            witnesses: FillWitnesses::default(),
        }
    }

    /// Complete the bags of `decomposition`, the tree the next rebuilds start
    /// from, in place of whatever was completed before.
    pub(super) fn complete(&mut self, decomposition: &TreeDecomposition) {
        self.completion.rows.fill(0);
        for bag in decomposition.bags() {
            let bag = bag.vertices();
            for (position, &left) in bag.iter().enumerate() {
                for &right in &bag[position + 1..] {
                    self.completion.insert(left as usize, right as usize);
                }
            }
        }
        self.witnesses.rebuild(&self.completion, &self.original);
    }
}

/// [`minimalization_candidate`] for `decomposition`, the tree the shared
/// completion was built from projected off `vertex` and compacted, over the
/// graph less `vertex` with `edge_count` edges. The work is charged as the
/// general pass charges it, so the two agree on every deadline they read.
pub(super) fn rebuild_candidate(
    decomposition: &TreeDecomposition,
    shared: &mut SharedCompletion,
    vertex: u32,
    edge_count: usize,
    deadline: Option<Instant>,
) -> Option<TreeDecomposition> {
    let vertices = decomposition.num_vertices() as usize;
    if vertices == 0 || !fits(decomposition, vertices, deadline) {
        return None;
    }
    charge_completion(decomposition, deadline)?;
    let completion = shared.completion.without_vertex(vertex as usize);
    let raise = |v: usize| v + usize::from(v >= vertex as usize);
    let original = &shared.original;
    let mut witnesses = RebuildWitnesses {
        store: &mut shared.witnesses,
        dropped: shared.completion.row(vertex as usize),
        vertex: vertex as usize,
        left: 0,
        cursor: 0,
        end: 0,
        slot: None,
    };
    refine_completion(
        completion,
        vertices,
        edge_count,
        |row, word| word_without(original.row(raise(row)), word, vertex as usize),
        &mut witnesses,
        deadline,
    )
}

/// The charges and deadline reads of [`completion`], without the inserts.
fn charge_completion(decomposition: &TreeDecomposition, deadline: Option<Instant>) -> Option<()> {
    let mut pacer = DeadlinePacer::new();
    for bag in decomposition.bags() {
        let bag = bag.vertices();
        crate::meter::charge((bag.len().saturating_mul(bag.len())) as u64);
        if pacer.due() && expired(deadline) {
            return None;
        }
    }
    Some(())
}

/// Drop the removable fill edges of `completion` and rebuild the bags.
fn refine_completion(
    mut completion: RowSet,
    vertices: usize,
    edge_count: usize,
    original_word: impl Fn(usize, usize) -> u64,
    witnesses: &mut impl Witnesses,
    deadline: Option<Instant>,
) -> Option<TreeDecomposition> {
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
    if minimalize(
        &mut completion,
        vertices,
        edge_count,
        original_word,
        witnesses,
        sweep_deadline,
    ) == 0
    {
        return None;
    }
    decompose_completion(&completion, vertices, deadline)
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
