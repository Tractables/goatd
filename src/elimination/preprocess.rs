//! Safe reduction rules for tree-decomposition preprocessing.
//!
//! Five fixed-point rules:
//!   - **islet**: a degree-0 vertex can be eliminated with bag `{v}`.
//!   - **twig**: a degree-1 vertex can be eliminated; its bag is {v, u}.
//!   - **series**: a degree-2 vertex with non-adjacent neighbours — add the
//!     fill edge, then eliminate; bag = {v, a, b}.
//!   - **simplicial**: a vertex whose live neighbours form a clique —
//!     eliminate with zero fill; bag = {v} ∪ N(v).
//!   - **almost-simplicial** (Bodlaender): a vertex `v` whose live neighbours
//!     form a clique except for one missing edge, *and* whose degree satisfies
//!     `deg(v) ≤ LB` for some valid lower bound `LB` on treewidth — here
//!     `LB = max deg over simplicial eliminations so far`. Adds one fill edge
//!     then eliminates; bag size = `deg(v)+1 ≤ LB+1`.
//!
//! Reattaching the recorded bags to a decomposition of the residual produces
//! a valid decomposition of the original graph. The widest recorded bag
//! captures the width contributed by removed vertices.

use std::time::Instant;

use super::execution::{DeadlinePacer, ElimSteps};
use super::graph::EliminationGraph;
use crate::deadline::expired;

/// The clock preprocessing runs against.
///
/// The rules run to a fixed point and the two clique tests are the
/// expensive part of them, so on a dense residual one pass can take seconds. Reduction is optional — whatever it has not done, the
/// elimination that follows does — so it stops when the caller's soft cutoff
/// arrives, or when the caller asks the solve to stop, and hands the rest of
/// the graph on.
pub(crate) struct PreprocessStop {
    deadline: Option<Instant>,
    pacer: DeadlinePacer,
    stopped: bool,
}

impl PreprocessStop {
    pub(crate) fn new(deadline: Option<Instant>) -> Self {
        Self {
            deadline,
            pacer: DeadlinePacer::new(),
            stopped: false,
        }
    }

    /// Whether the cutoff has arrived, asked at most once per millisecond of
    /// charged work.
    ///
    /// The pass with no deadline of its own asks as well, since `expired` is
    /// also where the caller's stop flag and an enclosing wall cutoff are
    /// read: a fixed-point scan over a large dense graph takes seconds, and
    /// nothing else would end it.
    #[inline]
    fn reached(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        if self.pacer.due() && expired(self.deadline) {
            self.stopped = true;
        }
        self.stopped
    }
}

/// Output of preprocessing. `Clone` so a single preprocess result can be
/// reused across multiple orders in the portfolio — preprocessing is
/// deterministic (no salt/seed) so sharing is safe.
#[derive(Clone)]
pub(crate) struct Reduced {
    /// Reduced graph (still holds inactive slots for eliminated vertices).
    pub graph: EliminationGraph,
    /// The eliminations the rules already did, which a run over `graph`
    /// continues from.
    pub prefix: ElimSteps,
}

pub(crate) fn preprocess(graph: EliminationGraph, deadline: Option<Instant>) -> Reduced {
    let mut stop = PreprocessStop::new(deadline);
    preprocess_with_stop(graph, &mut stop)
}

fn preprocess_with_stop(mut graph: EliminationGraph, stop: &mut PreprocessStop) -> Reduced {
    let mut prefix = ElimSteps::default();
    // Running lower bound on tw(G), maintained across simplicial/series
    // eliminations; gates the almost-simplicial rule below.
    let mut treewidth_lower_bound = 0usize;
    let mut marked = Marked::all(graph.len());
    // The bound the almost-simplicial rule last ran under.
    let mut almost_bound = 0usize;

    loop {
        // Exhaust degree-zero and degree-one vertices before any rule that can
        // emit a wider bag. One index-ordered twig scan is insufficient: a
        // high-index leaf can expose a lower-index leaf behind the cursor.
        // Reaching minimum degree two also establishes that any non-empty
        // residual component is not a forest before the series rule runs.
        let mut fired = peel_low_degree(&mut graph, &mut prefix, &mut marked);
        fired |= eliminate_series_vertices(
            &mut graph,
            &mut prefix,
            &mut marked,
            &mut treewidth_lower_bound,
            stop,
        );
        fired |= eliminate_simplicial_vertices(
            &mut graph,
            &mut prefix,
            &mut marked,
            &mut treewidth_lower_bound,
            stop,
        );
        // A higher bound admits vertices whose neighbourhoods did not change.
        if treewidth_lower_bound > almost_bound {
            marked.rules[ALMOST_SIMPLICIAL].mark_all();
            almost_bound = treewidth_lower_bound;
        }
        fired |= eliminate_almost_simplicial_vertices(
            &mut graph,
            &mut prefix,
            &mut marked,
            treewidth_lower_bound,
            stop,
        );

        if !fired || stop.reached() {
            break;
        }
    }

    Reduced { graph, prefix }
}

const LOW_DEGREE: usize = 0;
const SERIES: usize = 1;
const SIMPLICIAL: usize = 2;
const ALMOST_SIMPLICIAL: usize = 3;

/// The vertices each rule has still to test: all of them at first, then
/// those whose neighbourhood, or an edge within it, has changed since the
/// rule last tested them. A rule's eligibility depends on nothing else, so a
/// vertex it is not marked for would test as it did last time, and fail.
///
/// A pass takes its rule's marked vertices in index order from a cursor, so
/// one marked behind the cursor waits for the next pass, as it would in a
/// scan of every vertex: the rules fire on the same vertices in the same
/// order, and the passes cost the changes rather than the graph. On a chain
/// with pendant leaves each round of the rules removes a link or two, and the
/// scans made the reduction quadratic in its length.
struct Marked {
    rules: [MarkSet; 4],
}

impl Marked {
    fn all(n: usize) -> Self {
        Self {
            rules: std::array::from_fn(|_| MarkSet::all(n)),
        }
    }

    /// Mark `vertex` for every rule.
    fn mark(&mut self, vertex: u32) {
        for rule in &mut self.rules {
            rule.mark(vertex);
        }
    }

    /// Mark `vertex` and its live neighbours for every rule, after an edge
    /// at `vertex` was added: it changes the neighbourhood of `vertex` and
    /// adds an edge to the neighbourhoods its other end shares.
    fn mark_around(&mut self, graph: &EliminationGraph, vertex: u32, buf: &mut Vec<u32>) {
        self.mark(vertex);
        buf.clear();
        graph.collect_live_nbrs_into(vertex, buf);
        for &u in buf.iter() {
            self.mark(u);
        }
    }
}

/// A set of vertices as a bitset.
struct MarkSet {
    words: Vec<u64>,
    len: usize,
}

impl MarkSet {
    fn all(len: usize) -> Self {
        let mut set = Self {
            words: Vec::new(),
            len,
        };
        set.mark_all();
        set
    }

    fn mark_all(&mut self) {
        self.words.clear();
        self.words.resize(self.len.div_ceil(64), !0);
        if !self.len.is_multiple_of(64)
            && let Some(last) = self.words.last_mut()
        {
            *last = (1u64 << (self.len % 64)) - 1;
        }
    }

    fn mark(&mut self, vertex: u32) {
        self.words[vertex as usize / 64] |= 1 << (vertex % 64);
    }

    /// The least marked vertex at `from` or past it, taken out of the set.
    fn take_from(&mut self, from: usize) -> Option<u32> {
        let mut index = from / 64;
        let mut word = *self.words.get(index)? & (!0u64 << (from % 64));
        loop {
            if word != 0 {
                let bit = word.trailing_zeros() as usize;
                self.words[index] &= !(1u64 << bit);
                return Some((index * 64 + bit) as u32);
            }
            index += 1;
            word = *self.words.get(index)?;
        }
    }
}

/// The marked vertices of `rule` that are still in the graph, in index
/// order, each taken out of the set as the pass reaches it.
fn next_marked(
    marked: &mut Marked,
    rule: usize,
    cursor: &mut usize,
    graph: &EliminationGraph,
) -> Option<u32> {
    while let Some(vertex) = marked.rules[rule].take_from(*cursor) {
        *cursor = vertex as usize + 1;
        if graph.active[vertex as usize] {
            return Some(vertex);
        }
    }
    None
}

/// Eliminate one vertex and record its bag before the graph changes.
///
/// `needs_fill` false takes the removal that skips the fill pass, for a caller
/// that knows the neighbourhood is already a clique. Both paths take `vertex`
/// out of each neighbour's row the same way and adjust the edge count the same
/// way, and with no fill edge to add the fill pass writes nothing else, so the
/// graph is the same either way; what it saves is the k² membership test and
/// the per-neighbour stamping in the sparse path, and a pass over the row's
/// words per neighbour in bitset mode.
fn eliminate_and_record(
    graph: &mut EliminationGraph,
    prefix: &mut ElimSteps,
    marked: &mut Marked,
    vertex: u32,
    needs_fill: bool,
) -> usize {
    // The bag is collected first and the elimination takes a slice of it, so
    // the neighbour list is built once rather than into a vector the bag then
    // copies.
    let degree = graph.degree(vertex);
    let mut bag = Vec::with_capacity(degree + 1);
    bag.push(vertex);
    graph.collect_live_nbrs_into(vertex, &mut bag);
    debug_assert_eq!(bag.len(), degree + 1, "degree counts the live neighbours");
    if needs_fill {
        graph.eliminate_with_nbrs(vertex, &bag[1..]);
    } else {
        graph.remove_without_fill_nbrs(vertex, &bag[1..]);
    }
    for &u in &bag[1..] {
        marked.mark(u);
    }
    prefix.sink().record(vertex, bag);
    degree
}

fn eliminate_series_vertices(
    graph: &mut EliminationGraph,
    prefix: &mut ElimSteps,
    marked: &mut Marked,
    treewidth_lower_bound: &mut usize,
    stop: &mut PreprocessStop,
) -> bool {
    let mut fired = false;
    let (mut cursor, mut buf) = (0, Vec::new());
    while let Some(vertex) = next_marked(marked, SERIES, &mut cursor, graph) {
        if graph.degree(vertex) != 2 {
            continue;
        }
        if stop.reached() {
            return fired;
        }
        let neighbours = graph.live_neighbours(vertex);
        if !graph.contains_edge(neighbours[0], neighbours[1]) {
            // The rule fires exactly when the two neighbours are not adjacent,
            // so this elimination is the one that adds the edge between them.
            eliminate_and_record(graph, prefix, marked, vertex, true);
            marked.mark_around(graph, neighbours[0], &mut buf);
            marked.mark_around(graph, neighbours[1], &mut buf);
            *treewidth_lower_bound = (*treewidth_lower_bound).max(2);
            fired = true;
        }
    }
    fired
}

fn eliminate_simplicial_vertices(
    graph: &mut EliminationGraph,
    prefix: &mut ElimSteps,
    marked: &mut Marked,
    treewidth_lower_bound: &mut usize,
    stop: &mut PreprocessStop,
) -> bool {
    let mut fired = false;
    let mut cursor = 0;
    while let Some(vertex) = next_marked(marked, SIMPLICIAL, &mut cursor, graph) {
        if graph.degree(vertex) < 2 {
            continue;
        }
        if stop.reached() {
            return fired;
        }
        if graph.is_simplicial(vertex) {
            // `is_simplicial` has just established that the neighbourhood is a
            // clique, so there is no fill edge to add.
            let degree = eliminate_and_record(graph, prefix, marked, vertex, false);
            *treewidth_lower_bound = (*treewidth_lower_bound).max(degree);
            fired = true;
        }
    }
    fired
}

/// Apply the almost-simplicial rule where the known lower bound makes it safe.
fn eliminate_almost_simplicial_vertices(
    graph: &mut EliminationGraph,
    prefix: &mut ElimSteps,
    marked: &mut Marked,
    treewidth_lower_bound: usize,
    stop: &mut PreprocessStop,
) -> bool {
    if treewidth_lower_bound < 2 {
        return false;
    }

    let mut fired = false;
    let (mut cursor, mut buf) = (0, Vec::new());
    while let Some(vertex) = next_marked(marked, ALMOST_SIMPLICIAL, &mut cursor, graph) {
        let degree = graph.degree(vertex);
        if degree < 2 || degree > treewidth_lower_bound {
            continue;
        }
        if stop.reached() {
            return fired;
        }
        let Some((left, right)) = graph.almost_simplicial_nonedge(vertex) else {
            continue;
        };
        // The rule fires on exactly one missing edge, and the line above has
        // just added it, so the neighbourhood is now a clique.
        graph.add_edge(left, right);
        eliminate_and_record(graph, prefix, marked, vertex, false);
        marked.mark_around(graph, left, &mut buf);
        marked.mark_around(graph, right, &mut buf);
        fired = true;
    }
    fired
}

/// Eliminate degree-zero and degree-one vertices to a fixed point.
fn peel_low_degree(
    graph: &mut EliminationGraph,
    prefix: &mut ElimSteps,
    marked: &mut Marked,
) -> bool {
    let mut fired_any = false;
    loop {
        let mut fired = false;
        let mut cursor = 0;
        while let Some(v) = next_marked(marked, LOW_DEGREE, &mut cursor, graph) {
            match graph.degree(v) {
                0 => {
                    graph.deactivate_isolated(v);
                    prefix.sink().record(v, vec![v]);
                    fired = true;
                }
                1 => {
                    // A degree-one vertex has a one-vertex neighbourhood, so
                    // there is no fill edge to add.
                    eliminate_and_record(graph, prefix, marked, v, false);
                    fired = true;
                }
                _ => {}
            }
        }
        if !fired {
            return fired_any;
        }
        fired_any = true;
    }
}
