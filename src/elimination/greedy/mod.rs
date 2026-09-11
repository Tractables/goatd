//! Greedy min-fill and min-degree elimination cores.
//!
//! The deterministic cores use a seeded salt after their algorithmic keys.
//! The sampling cores draw from the full set tied on the primary key.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, hash_map::Entry};
use std::hash::{BuildHasherDefault, Hasher};
use std::time::Instant;

use super::execution::{Cutoff, DeadlinePacer, ElimExit, ElimSink, ElimStop, exceeds_width_bound};
use super::graph::{EliminationGraph, PreparedFill};
use crate::rng::Xorshift64;

/// Generates `Ord`/`PartialOrd` for a heap-entry struct that orders solely by
/// its `key` field (each slot `Reverse`-wrapped so minimums pop first on
/// Rust's max-heap).
#[cfg(test)]
macro_rules! ord_by_key {
    ($t:ty) => {
        impl Ord for $t {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.key.cmp(&other.key)
            }
        }
        impl PartialOrd for $t {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
    };
}

mod degree_queue;
mod deterministic;
mod min_degree;
mod min_fill;
mod sampling;

#[cfg(test)]
mod tests;

pub(super) use min_degree::eliminate_min_degree;
pub(super) use min_fill::eliminate_min_fill;
pub(super) use sampling::{
    SampleDraw, eliminate_sampled_fill_degree, eliminate_sampled_min_degree,
    eliminate_sampled_min_fill,
};

/// Above this many active vertices, a single cheap-mode eliminate on a dense
/// residual can overshoot the deadline by seconds. Emergency-bail immediately
/// on deadline rather than attempting cheap-mode elimination at this scale.
pub(super) const CHEAP_MODE_MAX_ACTIVE: usize = 512;

/// The graph-sized buffers one sampled elimination works in.
///
/// A portfolio runs a sampled core thousands of times over the same residual,
/// and a fresh set of these is a few hundred kilobytes of zeroed pages per
/// run — enough to be mapped and unmapped each time. The caller keeps one set
/// and hands it to every run instead. A run resets what it takes rather than
/// what it leaves, since a run that stops at its deadline stops in the middle
/// of its own state.
pub(super) struct SampleScratch {
    /// The marker the seeding scan counts fill from.
    fill: FillScratch,
    /// What each elimination reads for the scores it disturbs.
    affected: FillAffected,
    /// The priority buckets the draw picks from.
    buckets: BucketStorage,
    /// The live neighbours of the vertex being eliminated.
    live_nbrs: Vec<u32>,
    /// Each vertex's fill, for the scores that do not file it as the key.
    fills: Vec<u64>,
    /// Which degrees the min-degree sampler has still to re-file.
    degree_stale: Vec<bool>,
}

impl SampleScratch {
    /// Buffers for no graph at all: every run sizes them to its own.
    pub(super) fn new() -> Self {
        Self {
            fill: FillScratch::new(0),
            affected: FillAffected::new(0),
            buckets: BucketStorage::new(),
            live_nbrs: Vec::new(),
            fills: Vec::new(),
            degree_stale: Vec::new(),
        }
    }
}

/// Scratch state for min-fill's fill-count computation, reused across calls
/// to avoid reallocating per vertex.
pub(super) struct FillScratch {
    /// Stamp-based marker: `marker[v] == stamp` iff v ∈ current N(v) being
    /// checked. u16 (2 bytes/entry) instead of u32 (4 bytes/entry) halves the
    /// marker footprint — at N=16K the marker goes from 64KB (2×L1) to 32KB
    /// (fits L1), reducing random-access cache pressure in the fill-count
    /// inner loop. Wraparound at u16::MAX triggers a full `fill(0)` reset so
    /// stale stamp values from the previous cycle are never misread.
    marker: Vec<u16>,
    stamp: u16,
}

impl FillScratch {
    pub(super) fn new(n: usize) -> Self {
        FillScratch {
            marker: vec![0; n],
            stamp: 0,
        }
    }

    /// Make room for a graph of `n` vertices, keeping the marks that are
    /// there. A stamp is never 0, so the entries this adds read as unmarked.
    fn size_for(&mut self, n: usize) {
        if self.marker.len() < n {
            self.marker.resize(n, 0);
        }
    }

    #[inline]
    fn bump_stamp(&mut self) {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.marker.fill(0);
            self.stamp = 1;
        }
    }

    /// Count of fill edges needed to eliminate `v`. Counts edges inside N(v)
    /// via a stamp-marked array in O(|N(v)| + Σ deg(u) for u ∈ N(v)) time,
    /// avoiding an O(|N(v)|²) pair scan.
    ///
    /// Relies on `graph.adj` holding only active vertices — a graph invariant,
    /// not enforced here.
    pub(super) fn fill_count_of(&mut self, graph: &EliminationGraph, v: u32) -> u64 {
        // Bitset path is O(k · words) vs O(Σdeg) for the marker path below;
        // wins when avg_deg >> words ≈ n/64.
        if graph.bitset_words > 0 {
            crate::meter::charge(
                (graph.degree(v) as u64).saturating_mul(graph.bitset_words as u64),
            );
            return graph.fill_count_of_bs(v);
        }

        let nbrs_v = graph.adj[v as usize].as_slice();
        let k = nbrs_v.len();
        // The marker scan below is O(k + Σ deg(u) for u ∈ N(v)), and is charged
        // as that rather than as `k`: a lazy fill recompute on a dense residual
        // is one of the two places a min-fill candidate spends its budget, the
        // other being `EliminationGraph::eliminate_with_nbrs`. The summation
        // reads k row lengths, not the rows, so it is charged on every run and
        // not only under the meter: the seeding scan paces its deadline reads
        // by this figure, and on a graph whose degrees are as uneven as an
        // incidence graph's nothing cheaper stands in for it.
        let sigma: u64 = nbrs_v
            .iter()
            .map(|&u| graph.adj[u as usize].len() as u64)
            .sum();
        crate::meter::charge(k as u64 + sigma);
        if k < 2 {
            return 0;
        }

        self.bump_stamp();
        let s = self.stamp;

        for &u in nbrs_v {
            self.marker[u as usize] = s;
        }

        // Each edge in the induced subgraph is counted from both endpoints,
        // so `doubled` sums to 2× the true edge count.
        //
        // SAFETY: every index below is a vertex id in [0, graph.len()) — the
        // graph only ever stores ids it was built with, and elimination
        // deactivates vertices rather than renumbering or removing them, so
        // that bound holds for the whole run. `marker` has an entry for every
        // vertex of the graph the scratch was sized for, which is the graph it
        // is used with or a larger one it is being reused after; the
        // bounds-checked store into `marker[u]` a few lines above has already
        // proved that for every `u` this loop visits. Bounds checks cost ~30%
        // of this loop's instructions (a scalar gather on a u16 marker LLVM
        // won't vectorize).
        let mut doubled = 0u64;
        let marker = self.marker.as_ptr();
        for &u in nbrs_v {
            let adj_u = unsafe { graph.adj.get_unchecked(u as usize) };
            for &w in adj_u.iter() {
                // SAFETY: `w` is a vertex id, under the same bound as above.
                let m = unsafe { *marker.add(w as usize) };
                doubled += (m == s) as u64;
            }
        }
        let edge_count = doubled / 2;

        let total_pairs = (k as u64) * (k as u64 - 1) / 2;
        total_pairs - edge_count
    }
}

/// The fill scores an elimination disturbs, and what each of them needs.
///
/// Eliminating `v` changes two kinds of score. A vertex outside N(v) that is
/// adjacent to both ends of a fill edge loses one missing pair per such
/// edge; [`prepare`](Self::prepare) finds those and
/// [`pop_delta`](Self::pop_delta) hands them out. A neighbour `u` of `v`
/// changes in four terms, with A = N(u) \ {v} and B = N(v) \ {u}, all read
/// from the graph before the clique is filled in:
///
/// ```text
/// fill'(u) = fill(u) − nonadj(A ∩ B) + |A \ B|·|B \ A| − e(A \ B, B \ A) − |A \ B|
/// ```
///
/// nonadj(A ∩ B) is the number of `v`'s fill edges with both ends in N(u);
/// e(A \ B, B \ A) counts the edges between the neighbours `u` keeps and the
/// ones it gains, which, summed over the fill edges (u, b), is their common
/// neighbours outside N[v]. Both fall out of the one pass over the fill
/// edges that finds the outside deltas. |B \ A| is the number of fill edges
/// at `u`, which that same pass counts as it finds them, and |A \ B| follows
/// from it and the degrees; a neighbourhood known to be a clique has no
/// fill edges and reads no row at all. Before this the neighbours were
/// recounted from scratch after every elimination, at the degree times the
/// row width each, and that recount was most of a min-fill run.
///
/// The order vertices are first touched in is the order they are re-filed
/// in, which the sampled cores' draw depends on, so both passes visit the
/// fill edges and their common neighbours in the order the elimination
/// itself would have recorded them.
struct FillAffected {
    /// N(v): by slot in bitset mode, by vertex id otherwise.
    inside: Vec<u64>,
    /// Bitset mode only: the complement of `inside` with v's own slot
    /// cleared, so a fill edge's common neighbours split into inside and
    /// outside with one mask each and no per-word test for v.
    outside: Vec<u64>,
    /// Bitset mode only: the row of the neighbour whose fill edges are
    /// being read, masked by `inside` and by `outside`, so each of its
    /// fill edges reads one row and two masks rather than two rows and two
    /// masks.
    x_inside: Vec<u64>,
    x_outside: Vec<u64>,
    /// Bitset mode only: the words of v's row that hold a bit, and of the
    /// current neighbour's row split above, so a row hundreds of words wide
    /// with a few dozen bits is walked at the cost of its bits, not its
    /// width. Word order is kept, so the bits are visited in the order a
    /// full walk visits them.
    v_words: Vec<u32>,
    x_words: Vec<u32>,
    marker: Vec<u16>,
    stamp: u16,
    /// Fill decrease of a vertex outside N(v). Counted per bit of the
    /// common-neighbour words, so indexed the way the words are: by slot in
    /// bitset mode, by vertex id otherwise. `pop_delta` maps back once per
    /// vertex.
    delta: Vec<u64>,
    /// The indices with a non-zero `delta`, in first-touch order.
    vertices: Vec<u32>,
    /// For a neighbour `u` of `v`: |A \ B|, |B \ A|, nonadj(A ∩ B) and
    /// e(A \ B, B \ A). `inside_fill` is counted per bit and `cross` per
    /// fill edge, both indexed like `delta`; the set sizes are by vertex
    /// id. Read and reset by [`neighbour_fill`](Self::neighbour_fill).
    kept: Vec<u32>,
    gained: Vec<u32>,
    inside_fill: Vec<u64>,
    cross: Vec<u64>,
    /// Row mode: what each neighbour gains, its share of `partners` given
    /// by `starts`, and where v sits in its row, both by position in the
    /// neighbour list. What [`fill_edges`](Self::fill_edges) hands the
    /// elimination.
    partners: Vec<u32>,
    starts: Vec<u32>,
    v_position: Vec<u32>,
}

impl FillAffected {
    fn new(n: usize) -> Self {
        Self {
            inside: vec![0; n.div_ceil(64)],
            outside: vec![0; n.div_ceil(64)],
            x_inside: vec![0; n.div_ceil(64)],
            x_outside: vec![0; n.div_ceil(64)],
            v_words: Vec::new(),
            x_words: Vec::new(),
            marker: vec![0; n],
            stamp: 0,
            delta: vec![0; n],
            vertices: Vec::new(),
            kept: vec![0; n],
            gained: vec![0; n],
            inside_fill: vec![0; n],
            cross: vec![0; n],
            partners: Vec::new(),
            starts: Vec::new(),
            v_position: Vec::new(),
        }
    }

    /// Make room for a graph of `n` vertices, keeping what is there.
    ///
    /// Every counter is left at zero by the run that used it, so the entries
    /// this adds start where a fresh scratch's would.
    fn size_for(&mut self, n: usize) {
        let words = n.div_ceil(64);
        if self.inside.len() < words {
            self.inside.resize(words, 0);
            self.outside.resize(words, 0);
            self.x_inside.resize(words, 0);
            self.x_outside.resize(words, 0);
        }
        if self.marker.len() < n {
            self.marker.resize(n, 0);
            self.delta.resize(n, 0);
            self.kept.resize(n, 0);
            self.gained.resize(n, 0);
            self.inside_fill.resize(n, 0);
            self.cross.resize(n, 0);
        }
        debug_assert!(self.vertices.is_empty(), "a run leaves no deltas behind");
        debug_assert!(
            self.delta.iter().all(|&delta| delta == 0),
            "a run leaves no deltas behind"
        );
        debug_assert!(
            self.cross.iter().all(|&cross| cross == 0)
                && self.inside_fill.iter().all(|&fill| fill == 0),
            "a run leaves no neighbour terms behind"
        );
    }

    /// The fill edges the last successful [`prepare`](Self::prepare) with
    /// `fill` found, for `EliminationGraph::eliminate_prepared`.
    fn fill_edges(&self) -> PreparedFill<'_> {
        PreparedFill {
            gained: &self.gained,
            partners: &self.partners,
            starts: &self.starts,
            v_position: &self.v_position,
        }
    }

    fn clear(&mut self) {
        for &vertex in &self.vertices {
            self.delta[vertex as usize] = 0;
        }
        self.vertices.clear();
    }

    fn clear_neighbours(&mut self, graph: &EliminationGraph, nbrs: &[u32]) {
        for &u in nbrs {
            let index = Self::bit_index(graph, u);
            self.inside_fill[index] = 0;
            self.cross[index] = 0;
            let u = u as usize;
            self.kept[u] = 0;
            self.gained[u] = 0;
        }
    }

    /// The set sizes of every neighbour when N(v) is a clique: nothing is
    /// gained, and what is kept is the neighbour's degree less v and its
    /// other neighbours.
    fn prepare_clique(&mut self, graph: &EliminationGraph, nbrs: &[u32]) {
        let k = nbrs.len();
        crate::meter::charge(k as u64);
        for &u in nbrs {
            self.kept[u as usize] = (graph.degree(u) - k) as u32;
            self.gained[u as usize] = 0;
        }
    }

    /// Where the per-bit counters keep vertex `u`.
    #[inline]
    fn bit_index(graph: &EliminationGraph, u: u32) -> usize {
        if graph.bitset_words > 0 {
            graph.bitset_slot_of(u)
        } else {
            u as usize
        }
    }

    #[inline]
    fn bump_stamp(&mut self) {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.marker.fill(0);
            self.stamp = 1;
        }
    }

    /// One more missing pair filled for the vertex at `index`, which is a
    /// slot or a vertex id of the graph this scratch was sized for.
    #[inline]
    fn increment(&mut self, index: u32) {
        debug_assert!((index as usize) < self.delta.len());
        // SAFETY: the counters have one entry per vertex of the graph, which
        // is at least one per slot, and every index passed in is one or the
        // other, read from the graph's own rows.
        let delta = unsafe { self.delta.get_unchecked_mut(index as usize) };
        let first = *delta == 0;
        *delta += 1;
        if first {
            self.vertices.push(index);
        }
    }

    fn mark_inside(&mut self, vertex: u32) {
        let vertex = vertex as usize;
        self.inside[vertex >> 6] |= 1u64 << (vertex & 63);
    }

    fn unmark_inside(&mut self, vertex: u32) {
        let vertex = vertex as usize;
        self.inside[vertex >> 6] &= !(1u64 << (vertex & 63));
    }

    /// Row mode only: whether `vertex`, an id read from a row of the graph
    /// this scratch was sized for, is in N(v).
    #[inline]
    fn is_inside(&self, vertex: u32) -> bool {
        let vertex = vertex as usize;
        debug_assert!(vertex >> 6 < self.inside.len());
        // SAFETY: `inside` has a word for every 64 vertex ids of the graph.
        unsafe { *self.inside.get_unchecked(vertex >> 6) & (1u64 << (vertex & 63)) != 0 }
    }

    fn clear_inside(&mut self, nbrs: &[u32], bitset_words: usize) {
        if bitset_words > 0 {
            self.inside[..bitset_words].fill(0);
        } else {
            for &vertex in nbrs {
                self.unmark_inside(vertex);
            }
        }
    }

    fn prepare_inside(&mut self, graph: &EliminationGraph, v: u32, nbrs: &[u32]) {
        if graph.bitset_words > 0 {
            // In bitset mode `inside` is a copy of v's row, so it is indexed
            // by slot like the row is, and only its first `bitset_words` words
            // are in use — a bitset re-indexed over the residual is shorter
            // than one word per vertex of the graph.
            let words = graph.bitset_words;
            let start = graph.bitset_slot_of(v) * words;
            self.inside[..words].copy_from_slice(&graph.bitset[start..start + words]);
        } else {
            for &vertex in nbrs {
                self.mark_inside(vertex);
            }
        }
    }

    /// Gather everything the scores around `v`'s elimination need, from the
    /// graph as it stands before it: the outside deltas, and each
    /// neighbour's four terms. `nbrs` are `v`'s live neighbours. With `fill`
    /// false the neighbourhood is known to be a clique and only the set sizes
    /// are read. Returns false, with every scratch cleared, if `deadline`
    /// passes during the fill-edge pass; the caller still eliminates `v`.
    fn prepare(
        &mut self,
        graph: &EliminationGraph,
        v: u32,
        nbrs: &[u32],
        fill: bool,
        deadline: Option<Instant>,
    ) -> bool {
        debug_assert!(self.vertices.is_empty());
        self.prepare_inside(graph, v, nbrs);
        let done = if graph.bitset_words > 0 {
            self.prepare_bs(graph, v, nbrs, fill, deadline)
        } else {
            self.prepare_marker(graph, v, nbrs, fill, deadline)
        };
        self.clear_inside(nbrs, graph.bitset_words);
        if !done {
            self.clear();
            self.clear_neighbours(graph, nbrs);
        }
        done
    }

    fn prepare_bs(
        &mut self,
        graph: &EliminationGraph,
        v: u32,
        nbrs: &[u32],
        fill: bool,
        deadline: Option<Instant>,
    ) -> bool {
        let w = graph.bitset_words;
        let k = nbrs.len();
        if !fill {
            self.prepare_clique(graph, nbrs);
            return true;
        }
        let vi = graph.bitset_slot_of(v);
        // `inside` is v's row, which never carries v's own bit since the
        // graph holds no self-loop; `outside` is its complement without v.
        // Both are taken out of the scratch for the pass so the loops below
        // can hold them beside the counters.
        debug_assert!(
            self.inside[vi / 64] & (1u64 << (vi % 64)) == 0,
            "vertex {v} is its own neighbour"
        );
        let mut inside = std::mem::take(&mut self.inside);
        let mut outside = std::mem::take(&mut self.outside);
        let mut x_inside = std::mem::take(&mut self.x_inside);
        let mut x_outside = std::mem::take(&mut self.x_outside);
        let mut v_words = std::mem::take(&mut self.v_words);
        let mut x_words = std::mem::take(&mut self.x_words);
        v_words.clear();
        crate::meter::charge(w as u64);
        for (j, (out, &word)) in outside[..w].iter_mut().zip(&inside[..w]).enumerate() {
            *out = !word;
            if word != 0 {
                v_words.push(j as u32);
            }
        }
        outside[vi / 64] &= !(1u64 << (vi % 64));
        // The fill edges in the order the elimination would record them: by
        // neighbour, then by slot, each pair once from its smaller vertex.
        // Slots run in vertex-id order, so the slots say which is smaller.
        let mut done = true;
        let mut pacer = DeadlinePacer::new();
        'neighbours: for &u_raw in nbrs {
            let u = graph.bitset_slot_of(u_raw);
            let ub = u * w;
            crate::meter::charge(v_words.len() as u64);
            // u's fill edges are v's other neighbours that u's row lacks:
            // u's own bit leaves v's row while they are read, and is put
            // back after. u is never a common neighbour of its own fill
            // edge, so the pass over each edge does not see the difference.
            inside[u / 64] &= !(1u64 << (u % 64));
            let u_row = &graph.bitset[ub..ub + w];
            let mut gained = 0u32;
            let mut masked = false;
            // u's own word may have gone empty above; walking it is harmless.
            for &j in &v_words {
                let j = j as usize;
                debug_assert!(j < w);
                // SAFETY: `v_words` holds indices below `w`, and `inside`
                // and u's row both hold `w` words.
                let (inside_word, u_word) =
                    unsafe { (*inside.get_unchecked(j), *u_row.get_unchecked(j)) };
                let mut mask = inside_word & !u_word;
                while mask != 0 {
                    let other = j * 64 + mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    gained += 1;
                    if u < other {
                        if pacer.due() && crate::deadline::expired(deadline) {
                            done = false;
                            break 'neighbours;
                        }
                        if !masked {
                            // u's row split by v's, once for all the fill
                            // edges read from u, keeping the words that
                            // hold a bit of either part. The words left
                            // stale are never read.
                            masked = true;
                            crate::meter::charge(w as u64);
                            x_words.clear();
                            let rows = u_row.iter().zip(&inside[..w]).zip(&outside[..w]);
                            let splits = x_inside[..w].iter_mut().zip(x_outside[..w].iter_mut());
                            for (j, (((&x_word, &in_word), &out_word), (x_in, x_out))) in
                                rows.zip(splits).enumerate()
                            {
                                let split_in = x_word & in_word;
                                let split_out = x_word & out_word;
                                if (split_in | split_out) != 0 {
                                    x_words.push(j as u32);
                                    *x_in = split_in;
                                    *x_out = split_out;
                                }
                            }
                        }
                        self.fill_edge_bs(
                            graph,
                            &x_words,
                            &x_inside[..w],
                            &x_outside[..w],
                            u,
                            other,
                        );
                    }
                }
            }
            inside[u / 64] |= 1u64 << (u % 64);
            // The fill edges at u are B \ A. A ∩ B is the rest of B, and
            // A \ B the rest of A, whose size is u's degree less v.
            self.gained[u_raw as usize] = gained;
            self.kept[u_raw as usize] = (graph.degree(u_raw) + gained as usize - k) as u32;
        }
        self.inside = inside;
        self.outside = outside;
        self.x_inside = x_inside;
        self.x_outside = x_outside;
        self.v_words = v_words;
        self.x_words = x_words;
        done
    }

    /// One fill edge of v between the slots `x` and `y`, both in N(v), with
    /// x's row already split into `x_inside` and `x_outside` by v's and
    /// `x_words` naming the words of the split that hold a bit: its common
    /// neighbours inside N(v) each lose one missing pair from A ∩ B, those
    /// outside lose one from their own fill, and their number is what x and
    /// y each gain in e(A \ B, B \ A).
    #[inline]
    fn fill_edge_bs(
        &mut self,
        graph: &EliminationGraph,
        x_words: &[u32],
        x_inside: &[u64],
        x_outside: &[u64],
        x: usize,
        y: usize,
    ) {
        let w = graph.bitset_words;
        crate::meter::charge(x_words.len() as u64);
        let ys = y * w;
        let mut out = 0u64;
        debug_assert!(ys + w <= graph.bitset.len());
        // SAFETY: y is a slot read from the bitset's own rows, and the
        // bitset holds `w` words for every slot.
        let y_row = unsafe { graph.bitset.get_unchecked(ys..ys + w) };
        for &word in x_words {
            let word = word as usize;
            debug_assert!(word < w && word < x_inside.len() && word < x_outside.len());
            // SAFETY: `x_words` holds indices below `w`, and y's row and
            // both halves of the split hold `w` words.
            let (y_word, x_in, x_out) = unsafe {
                (
                    *y_row.get_unchecked(word),
                    *x_inside.get_unchecked(word),
                    *x_outside.get_unchecked(word),
                )
            };
            let mut ins = y_word & x_in;
            while ins != 0 {
                let slot = word * 64 + ins.trailing_zeros() as usize;
                // SAFETY: a set bit of a row is a slot the bitset was built
                // with, and the counters have one entry per vertex of the
                // graph, which is at least one per slot.
                unsafe { *self.inside_fill.get_unchecked_mut(slot) += 1 };
                ins &= ins - 1;
            }
            let mut outs = y_word & x_out;
            while outs != 0 {
                let slot = word * 64 + outs.trailing_zeros() as usize;
                self.increment(slot as u32);
                out += 1;
                outs &= outs - 1;
            }
        }
        self.cross[x] += out;
        self.cross[y] += out;
    }

    fn prepare_marker(
        &mut self,
        graph: &EliminationGraph,
        v: u32,
        nbrs: &[u32],
        fill: bool,
        deadline: Option<Instant>,
    ) -> bool {
        let k = nbrs.len();
        if !fill {
            self.prepare_clique(graph, nbrs);
            return true;
        }
        let mut pacer = DeadlinePacer::new();
        self.partners.clear();
        self.starts.clear();
        self.v_position.clear();
        for &u in nbrs {
            // Stamp u's row: it says which of v's other neighbours u lacks,
            // which are its fill edges, and stays valid while they are
            // processed since nothing below stamps again. The same walk
            // finds v, which the elimination takes out of the row.
            self.bump_stamp();
            let stamp = self.stamp;
            let row = &graph.adj[u as usize];
            crate::meter::charge((row.len() + k) as u64);
            let mut v_position = 0usize;
            for (position, &z) in row.iter().enumerate() {
                debug_assert!((z as usize) < self.marker.len());
                // SAFETY: a row holds vertex ids of the graph this scratch was
                // sized for, and `marker` has an entry per vertex.
                unsafe { *self.marker.get_unchecked_mut(z as usize) = stamp };
                if z == v {
                    v_position = position;
                }
            }
            debug_assert_eq!(row[v_position], v, "{u} is not a neighbour of {v}");
            self.v_position.push(v_position as u32);
            let start = self.partners.len();
            self.starts.push(start as u32);
            // The fill edges in the order the elimination would record them:
            // by neighbour, then by the neighbour list, each pair once from
            // its smaller vertex.
            for &y in nbrs {
                if y != u && self.marker[y as usize] != stamp {
                    self.partners.push(y);
                    if u < y {
                        if pacer.due() && crate::deadline::expired(deadline) {
                            return false;
                        }
                        self.fill_edge_marker(graph, v, u, y, stamp);
                    }
                }
            }
            // As in the bitset pass: the fill edges at u are B \ A, and A \ B
            // is the rest of u's row less v.
            let gained = self.partners.len() - start;
            self.gained[u as usize] = gained as u32;
            self.kept[u as usize] = (row.len() + gained - k) as u32;
        }
        self.starts.push(self.partners.len() as u32);
        true
    }

    /// The sparse form of [`fill_edge_bs`](Self::fill_edge_bs), with x's row
    /// stamped. y's row is walked as the elimination would leave it, with the
    /// last entry moved into v's place, so the outside vertices are first
    /// touched in the order they always were.
    #[inline]
    fn fill_edge_marker(&mut self, graph: &EliminationGraph, v: u32, x: u32, y: u32, stamp: u16) {
        let row = &graph.adj[y as usize];
        crate::meter::charge(row.len() as u64);
        let last = row.len() - 1;
        let mut out = 0u64;
        for &entry in &row[..last] {
            let z = if entry == v { row[last] } else { entry };
            debug_assert!((z as usize) < self.marker.len());
            // SAFETY: as in `prepare_marker`, a row entry is a vertex id and
            // the scratch has an entry per vertex.
            if unsafe { *self.marker.get_unchecked(z as usize) } != stamp {
                continue;
            }
            if self.is_inside(z) {
                self.inside_fill[z as usize] += 1;
            } else {
                self.increment(z);
                out += 1;
            }
        }
        self.cross[x as usize] += out;
        self.cross[y as usize] += out;
    }

    /// The next vertex outside N(v) whose fill changed, and by how much, in
    /// reverse first-touch order.
    fn pop_delta(&mut self, graph: &EliminationGraph) -> Option<(u32, u64)> {
        self.vertices.pop().map(|index| {
            let delta = std::mem::replace(&mut self.delta[index as usize], 0);
            let vertex = if graph.bitset_words > 0 {
                graph.bitset_vertex_at(index as usize)
            } else {
                index
            };
            (vertex, delta)
        })
    }

    /// The fill of `u`, a neighbour of the vertex [`prepare`](Self::prepare)
    /// was given, after that elimination, from its fill `old` before it.
    /// Consumes u's terms.
    #[inline]
    fn neighbour_fill(&mut self, graph: &EliminationGraph, u: u32, old: u64) -> u64 {
        let index = Self::bit_index(graph, u);
        let inside_fill = std::mem::take(&mut self.inside_fill[index]);
        let cross = std::mem::take(&mut self.cross[index]);
        let u = u as usize;
        let kept = u64::from(std::mem::take(&mut self.kept[u]));
        let gained = u64::from(std::mem::take(&mut self.gained[u]));
        let added = kept * gained;
        let removed = inside_fill + cross + kept;
        debug_assert!(old + added >= removed, "fill of {u} would go negative");
        old + added - removed
    }
}

/// Snapshot `v`'s live neighbours into `nbrs_buf` and build the bag its
/// elimination emits: `v` first, then those neighbours.
fn take_bag(graph: &EliminationGraph, v: u32, nbrs_buf: &mut Vec<u32>) -> Vec<u32> {
    nbrs_buf.clear();
    graph.collect_live_nbrs_into(v, nbrs_buf);
    let mut bag = Vec::with_capacity(nbrs_buf.len() + 1);
    bag.push(v);
    bag.extend_from_slice(nbrs_buf);
    bag
}

/// What every elimination heap entry can be asked, whatever its ordering key:
/// which vertex it stands for, and the score it recorded when it was pushed.
trait ElimEntry {
    fn vertex(&self) -> u32;
    /// The fill or degree at push time. The skeleton compares it against a
    /// fresh measurement to spot an entry whose ordering key no longer holds.
    fn snapshot(&self) -> u64;
}

/// One priority key's vertices, in the order they were filed, and their
/// combined sampling mass. The mass stays 0 when every weight is the same,
/// where it follows from the vertex count instead.
struct Bucket {
    vertices: Vec<u32>,
    sampling_mass: u64,
}

/// Hashes internal `u64` priority keys without the cost of general-purpose
/// keyed hashing.
#[derive(Clone, Default)]
struct PriorityHasher(u64);

impl Hasher for PriorityHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = 0u64;
        for &byte in bytes {
            hash = hash.rotate_left(8) ^ u64::from(byte);
        }
        self.0 = hash;
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

type PriorityHashMap = HashMap<u64, Bucket, BuildHasherDefault<PriorityHasher>>;

/// How many priority keys per vertex get a direct slot. A fill-based score is
/// a fill count plus, for the diverse orders, up to `32 * n` from the degree
/// coefficient, so this covers the keys those cores produce without a hash.
const DENSE_KEYS_PER_VERTEX: usize = 64;

/// The most keys that ever get a slot. Slots are allocated lazily up to the
/// largest key seen, and this keeps the array to a few megabytes on a graph
/// with hundreds of thousands of vertices.
const MAX_DENSE_PRIORITY_KEYS: usize = 1 << 20;

/// A dense slot holding no bucket. Any other value indexes `buckets`.
const NO_BUCKET: u32 = u32::MAX;

/// Priority key → bucket. Keys below `dense_keys` get a slot in `slots`,
/// which indexes bucket storage rather than holding a bucket inline so that
/// growing the slot array costs four bytes a key. A fill count can reach
/// n²/2, well past any slot array worth allocating, so keys at or above
/// `dense_keys` go to `overflow` instead.
struct PriorityBuckets {
    slots: Vec<u32>,
    buckets: Vec<Bucket>,
    /// Bucket storage that no key names any more, waiting to be handed out.
    free: Vec<u32>,
    dense_keys: usize,
    overflow: PriorityHashMap,
}

impl PriorityBuckets {
    fn new(vertex_count: usize) -> Self {
        PriorityBuckets {
            slots: Vec::new(),
            buckets: Vec::new(),
            free: Vec::new(),
            dense_keys: Self::dense_keys_for(vertex_count),
            overflow: PriorityHashMap::default(),
        }
    }

    /// How many keys get a slot on a graph of `vertex_count` vertices.
    fn dense_keys_for(vertex_count: usize) -> usize {
        vertex_count
            .saturating_mul(DENSE_KEYS_PER_VERTEX)
            .clamp(256, MAX_DENSE_PRIORITY_KEYS)
    }

    #[inline]
    fn dense_index(&self, key: u64) -> Option<usize> {
        usize::try_from(key)
            .ok()
            .filter(|&key| key < self.dense_keys)
    }

    fn get(&self, key: u64) -> Option<&Bucket> {
        match self.dense_index(key) {
            Some(key) => match self.slots.get(key) {
                Some(&slot) if slot != NO_BUCKET => Some(&self.buckets[slot as usize]),
                _ => None,
            },
            None => self.overflow.get(&key),
        }
    }

    #[inline]
    fn get_mut(&mut self, key: u64) -> Option<&mut Bucket> {
        match self.dense_index(key) {
            Some(key) => match self.slots.get(key) {
                Some(&slot) if slot != NO_BUCKET => Some(&mut self.buckets[slot as usize]),
                _ => None,
            },
            None => self.overflow.get_mut(&key),
        }
    }

    /// The bucket `key` names, created empty if it does not exist yet. The
    /// flag says whether this call created it, which is what moves the
    /// tracked minimum. A created bucket takes its vertex storage from
    /// `spare` when one is waiting there.
    #[inline]
    fn get_or_insert(&mut self, key: u64, spare: &mut Vec<Vec<u32>>) -> (&mut Bucket, bool) {
        let Some(index) = self.dense_index(key) else {
            return match self.overflow.entry(key) {
                Entry::Occupied(entry) => (entry.into_mut(), false),
                Entry::Vacant(entry) => (entry.insert(Self::empty_bucket(spare)), true),
            };
        };
        if index >= self.slots.len() {
            self.slots.resize(index + 1, NO_BUCKET);
        }
        let mut created = false;
        if self.slots[index] == NO_BUCKET {
            created = true;
            self.slots[index] = match self.free.pop() {
                Some(slot) => slot,
                None => {
                    self.buckets.push(Self::empty_bucket(spare));
                    u32::try_from(self.buckets.len() - 1).expect("bucket count fits u32")
                }
            };
        }
        (&mut self.buckets[self.slots[index] as usize], created)
    }

    fn empty_bucket(spare: &mut Vec<Vec<u32>>) -> Bucket {
        Bucket {
            vertices: spare.pop().unwrap_or_default(),
            sampling_mass: 0,
        }
    }

    /// Drop the bucket `key` names, which has to be empty. Dense storage goes
    /// back on the free list with its vertex allocation; an overflow bucket
    /// hands that allocation to `spare`.
    #[inline]
    fn remove_empty(&mut self, key: u64, spare: &mut Vec<Vec<u32>>) {
        match self.dense_index(key) {
            Some(index) => {
                let slot = std::mem::replace(&mut self.slots[index], NO_BUCKET);
                debug_assert!(slot != NO_BUCKET, "bucket missing");
                debug_assert!(self.buckets[slot as usize].vertices.is_empty());
                debug_assert_eq!(self.buckets[slot as usize].sampling_mass, 0);
                self.free.push(slot);
            }
            None => {
                let bucket = self.overflow.remove(&key).expect("bucket missing");
                spare.push(bucket.vertices);
            }
        }
    }

    /// The smallest live key, scanning the slots from `from` upwards. Callers
    /// pass a key no live bucket sits below.
    fn smallest_key_from(&self, from: u64) -> Option<u64> {
        let start = usize::try_from(from)
            .unwrap_or(usize::MAX)
            .min(self.slots.len());
        if let Some(offset) = self.slots[start..]
            .iter()
            .position(|&slot| slot != NO_BUCKET)
        {
            return Some((start + offset) as u64);
        }
        self.overflow.keys().copied().min()
    }
}

#[derive(Clone, Copy)]
struct BucketPosition {
    key: u64,
    index: usize,
}

impl BucketPosition {
    const VACANT: Self = Self {
        key: 0,
        index: usize::MAX,
    };

    fn is_vacant(self) -> bool {
        self.index == usize::MAX
    }
}

/// The storage a [`BucketMap`] files vertices in.
///
/// It is the caller's, not the map's, so that a portfolio's restarts refill
/// one set of buckets rather than allocating a position array and a bucket
/// per key on every run.
pub(super) struct BucketStorage {
    buckets: PriorityBuckets,
    spare_vertices: Vec<Vec<u32>>,
    position: Vec<BucketPosition>,
}

impl BucketStorage {
    /// Storage for no graph at all: [`Self::reset`] sizes it to each run's.
    pub(super) fn new() -> Self {
        Self {
            buckets: PriorityBuckets::new(0),
            spare_vertices: Vec::new(),
            position: Vec::new(),
        }
    }

    /// Empty the storage and size it for a graph of `vertex_count` vertices.
    ///
    /// A run that stopped at its deadline leaves its remaining vertices filed,
    /// so the buckets are emptied here rather than assumed empty. A run that
    /// finished has already given every bucket back, which is why the sweep
    /// over the slots only happens when one is still out.
    fn reset(&mut self, vertex_count: usize) {
        let live =
            self.buckets.buckets.len() - self.buckets.free.len() + self.buckets.overflow.len();
        if live > 0 {
            self.buckets.slots.fill(NO_BUCKET);
            for bucket in &mut self.buckets.buckets {
                bucket.vertices.clear();
                bucket.sampling_mass = 0;
            }
            self.buckets.free.clear();
            let count = u32::try_from(self.buckets.buckets.len()).expect("bucket count fits u32");
            self.buckets.free.extend(0..count);
            for (_, mut bucket) in self.buckets.overflow.drain() {
                // A bucket only reaches the spare pool empty: what comes out
                // of it becomes another key's bucket as it stands.
                bucket.vertices.clear();
                self.spare_vertices.push(bucket.vertices);
            }
        }
        debug_assert!(
            self.spare_vertices.iter().all(|spare| spare.is_empty()),
            "spare bucket storage holds no vertices"
        );
        self.buckets.dense_keys = PriorityBuckets::dense_keys_for(vertex_count);
        // Every slot is free by now, so the ones a wider graph left behind
        // hold nothing and can go.
        self.buckets.slots.truncate(self.buckets.dense_keys);
        self.position.clear();
        self.position.resize(vertex_count, BucketPosition::VACANT);
    }
}

pub(super) struct BucketMap<'a> {
    buckets: &'a mut PriorityBuckets,
    /// Exact while `minimum_dirty` is false. Removing the current minimum
    /// marks it dirty; the next read scans the live priority keys once.
    minimum_key: Option<u64>,
    minimum_dirty: bool,
    spare_vertices: &'a mut Vec<Vec<u32>>,
    position: &'a mut Vec<BucketPosition>,
    weights: &'a [u32],
    uniform_mass: Option<u64>,
}

impl<'a> BucketMap<'a> {
    /// An empty map over `storage`, which it empties and sizes for the graph
    /// `weights` covers.
    fn with_weights(
        storage: &'a mut BucketStorage,
        weights: &'a [u32],
        uniform_mass: Option<u64>,
    ) -> Self {
        storage.reset(weights.len());
        let BucketStorage {
            buckets,
            spare_vertices,
            position,
        } = storage;
        BucketMap {
            buckets,
            minimum_key: None,
            minimum_dirty: false,
            spare_vertices,
            position,
            weights,
            uniform_mass,
        }
    }

    /// File `v` under `key`. Inlined, as `remove_at` is, into `update`: the
    /// score repair calls that once per changed vertex, and a move between
    /// buckets is then one function rather than three.
    #[inline(always)]
    fn insert(&mut self, v: u32, key: u64) {
        let (bucket, created) = self.buckets.get_or_insert(key, self.spare_vertices);
        let idx = bucket.vertices.len();
        bucket.vertices.push(v);
        if self.uniform_mass.is_none() {
            bucket.sampling_mass += sampling_mass(self.weights[v as usize]);
        }
        if created {
            self.minimum_key = Some(self.minimum_key.map_or(key, |minimum| minimum.min(key)));
        }
        debug_assert!(self.position[v as usize].is_vacant());
        self.position[v as usize] = BucketPosition { key, index: idx };
    }

    fn remove_vertex(&mut self, v: u32) {
        let position = std::mem::replace(&mut self.position[v as usize], BucketPosition::VACANT);
        if !position.is_vacant() {
            self.remove_at(v, position);
        }
    }

    #[inline(always)]
    fn remove_at(&mut self, v: u32, position: BucketPosition) {
        let bucket = self.buckets.get_mut(position.key).expect("bucket missing");
        if Self::remove_from_bucket(
            bucket,
            self.position.as_mut_slice(),
            self.weights,
            self.uniform_mass,
            v,
            position,
        ) {
            self.buckets.remove_empty(position.key, self.spare_vertices);
            self.minimum_dirty |= self.minimum_key == Some(position.key);
        }
    }

    #[inline(always)]
    fn remove_from_bucket(
        bucket: &mut Bucket,
        positions: &mut [BucketPosition],
        weights: &[u32],
        uniform_mass: Option<u64>,
        v: u32,
        position: BucketPosition,
    ) -> bool {
        if uniform_mass.is_none() {
            bucket.sampling_mass -= sampling_mass(weights[v as usize]);
        }
        let last_idx = bucket.vertices.len() - 1;
        if position.index != last_idx {
            let moved = bucket.vertices[last_idx];
            bucket.vertices[position.index] = moved;
            positions[moved as usize] = BucketPosition {
                key: position.key,
                index: position.index,
            };
        }
        bucket.vertices.pop();
        bucket.vertices.is_empty()
    }

    #[inline]
    fn update(&mut self, v: u32, new_key: u64) {
        let position = self.position[v as usize];
        if !position.is_vacant() && position.key == new_key {
            return;
        }
        if !position.is_vacant() {
            self.position[v as usize] = BucketPosition::VACANT;
            self.remove_at(v, position);
        }
        self.insert(v, new_key);
    }

    /// The smallest live priority key, rescanning once when the last removal
    /// emptied the bucket it named.
    fn minimum(&mut self) -> Option<u64> {
        if self.minimum_dirty {
            self.minimum_key = self
                .buckets
                .smallest_key_from(self.minimum_key.unwrap_or(0));
            self.minimum_dirty = false;
        }
        self.minimum_key
    }

    /// The live buckets of a tie set: those whose key is within `band` of
    /// `minimum`, ascending by key. Concatenating their vertices, each bucket
    /// in storage order, gives the tie set the samplers draw from, so the tie
    /// set is a function of the map's history alone.
    fn band_buckets(&self, minimum: u64, band: u64) -> impl Iterator<Item = &Bucket> {
        (minimum..=minimum.saturating_add(band)).filter_map(|key| self.bucket(key))
    }

    /// Pick one vertex from the tie set within `band` of the minimum, giving
    /// smaller weights more mass. A tie set holding a single vertex draws
    /// nothing at all, so the RNG stream depends only on the ties the
    /// elimination actually had to break.
    ///
    /// The buckets are walked in place: the tie set is never materialised, and
    /// on a wide band that is the difference between a copy of every tied
    /// vertex per elimination and a walk that stops at the drawn one.
    fn sample_min_band(&mut self, band: u64, rng: &mut Xorshift64) -> Option<u32> {
        let minimum = self.minimum()?;
        let mut total_mass = 0;
        let mut tied = 0;
        let mut first = 0;
        for bucket in self.band_buckets(minimum, band) {
            if tied == 0
                && let Some(&head) = bucket.vertices.first()
            {
                first = head;
            }
            tied += bucket.vertices.len();
            total_mass += self.bucket_mass(bucket);
        }
        debug_assert!(tied > 0, "a live minimum has a nonempty bucket");
        debug_assert!(total_mass > 0, "a tie set's vertices each carry mass");
        if tied == 1 {
            return Some(first);
        }
        // Compose two u32 draws into one u64 so the draw covers `total_mass` up
        // to 2^64.
        let hi = rng.next_u32() as u64;
        let lo = rng.next_u32() as u64;
        let r = ((hi << 32) | lo) % total_mass;
        if let Some(mass) = self.uniform_mass {
            let mut index = (r / mass) as usize;
            crate::meter::charge(1);
            for bucket in self.band_buckets(minimum, band) {
                if let Some(&v) = bucket.vertices.get(index) {
                    return Some(v);
                }
                index -= bucket.vertices.len();
            }
            unreachable!("a mass below the total names a tied vertex");
        }
        // The chosen vertex is tracked rather than returned from inside the
        // walk, so that the scan can be charged on the way out. Falling out of
        // the walk leaves the last tied vertex, the same fall-through the walk
        // has always had: `r` is reduced modulo the total, so the final partial
        // sum always covers it and the walk is expected to break.
        let mut acc: u64 = 0;
        let mut scanned = 0;
        let mut pick = first;
        'walk: for bucket in self.band_buckets(minimum, band) {
            for &v in &bucket.vertices {
                acc += sampling_mass(self.weights[v as usize]);
                scanned += 1;
                pick = v;
                if r < acc {
                    break 'walk;
                }
            }
        }
        // Bucket membership updates maintain the total mass. The remaining
        // prefix walk is the dominant cost for nonuniform weights, so charge
        // its touches at the same rate as graph touches.
        crate::meter::charge(scanned);
        Some(pick)
    }

    /// A bucket's sampling mass, which equal weights leave as a count.
    fn bucket_mass(&self, bucket: &Bucket) -> u64 {
        self.uniform_mass.map_or(bucket.sampling_mass, |mass| {
            mass * bucket.vertices.len() as u64
        })
    }

    fn key_of(&self, v: u32) -> Option<u64> {
        let position = self.position[v as usize];
        (!position.is_vacant()).then_some(position.key)
    }

    fn bucket(&self, key: u64) -> Option<&Bucket> {
        self.buckets.get(key)
    }
}

/// Fill counts for every active vertex via adj-based `FillScratch`, so the
/// O(n·d²) computation can be cached once and reused across multiple seeds.
pub(super) fn compute_initial_fill(graph: &EliminationGraph) -> Vec<u64> {
    let n = graph.len();
    let mut scratch = FillScratch::new(n);
    (0..n)
        .map(|v| {
            if graph.active[v] {
                scratch.fill_count_of(graph, v as u32)
            } else {
                0
            }
        })
        .collect()
}

/// Sampling mass for a public, earlier-first weight. Adding one after the
/// inversion keeps `u32::MAX` reachable with mass 1.
#[inline]
fn sampling_mass(earlier_first_weight: u32) -> u64 {
    u64::from(u32::MAX - earlier_first_weight) + 1
}

/// The common mass when every public weight is equal. Detect it once per
/// elimination order so uniform tie sets do not need repeated weight scans.
fn uniform_sampling_mass(weights: &[u32]) -> Option<u64> {
    let (&first, rest) = weights.split_first()?;
    let mut scanned = 1u64;
    for &weight in rest {
        scanned += 1;
        if weight != first {
            crate::meter::charge(scanned);
            return None;
        }
    }
    crate::meter::charge(scanned);
    Some(sampling_mass(first))
}
