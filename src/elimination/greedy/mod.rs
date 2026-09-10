//! Greedy min-fill and min-degree elimination cores.
//!
//! The deterministic cores use a seeded salt after their algorithmic keys.
//! The sampling cores draw from the full set tied on the primary key.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, hash_map::Entry};
use std::hash::{BuildHasherDefault, Hasher};
use std::time::Instant;

use super::execution::{Cutoff, DeadlinePacer, ElimExit, ElimSink, ElimStop, exceeds_width_bound};
use super::graph::EliminationGraph;
use crate::rng::Xorshift64;

/// Generates `Ord`/`PartialOrd` for a heap-entry struct that orders solely by
/// its `key` field (each slot `Reverse`-wrapped so minimums pop first on
/// Rust's max-heap).
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
        // that bound holds for the whole run. `marker` is allocated to
        // `graph.len()` by `FillScratch::new` and the scratch is built from
        // the same graph it is used with, so the two lengths agree; the
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
/// edges that finds the outside deltas, and the two set sizes are one
/// masked popcount, or one row scan, per neighbour. Before this the
/// neighbours were recounted from scratch after every elimination, at the
/// degree times the row width each, and that recount was most of a min-fill
/// run.
///
/// The order vertices are first touched in is the order they are re-filed
/// in, which the sampled cores' draw depends on, so both passes visit the
/// fill edges and their common neighbours in the order the elimination
/// itself would have recorded them.
struct FillAffected {
    /// N(v): by slot in bitset mode, by vertex id otherwise.
    inside: Vec<u64>,
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
    /// e(A \ B, B \ A). `inside_fill` is counted per bit and indexed like
    /// `delta`; the others are by vertex id. Read and reset by
    /// [`neighbour_fill`](Self::neighbour_fill).
    kept: Vec<u32>,
    gained: Vec<u32>,
    inside_fill: Vec<u64>,
    cross: Vec<u64>,
}

impl FillAffected {
    fn new(n: usize) -> Self {
        Self {
            inside: vec![0; n.div_ceil(64)],
            marker: vec![0; n],
            stamp: 0,
            delta: vec![0; n],
            vertices: Vec::new(),
            kept: vec![0; n],
            gained: vec![0; n],
            inside_fill: vec![0; n],
            cross: vec![0; n],
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
            self.inside_fill[Self::bit_index(graph, u)] = 0;
            let u = u as usize;
            self.kept[u] = 0;
            self.gained[u] = 0;
            self.cross[u] = 0;
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

    #[inline]
    fn increment(&mut self, index: u32) {
        let delta = &mut self.delta[index as usize];
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

    fn is_inside(&self, vertex: u32) -> bool {
        let vertex = vertex as usize;
        self.inside[vertex >> 6] & (1u64 << (vertex & 63)) != 0
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
        let vi = graph.bitset_slot_of(v);
        let vb = vi * w;
        for &u in nbrs {
            // Both counts include v's own bit in u's row, which the fill
            // count never sees.
            crate::meter::charge(w as u64);
            let kept = graph.bitset_difference_count(u, v) - 1;
            self.kept[u as usize] = kept as u32;
            // |B \ A| = |B| − |A ∩ B|, both counted without u and v.
            self.gained[u as usize] = (k as u64 + kept - graph.degree(u) as u64) as u32;
        }
        if !fill {
            return true;
        }
        // The fill edges in the order the elimination would record them: by
        // neighbour, then by slot, each pair once from its smaller vertex.
        let mut pacer = DeadlinePacer::new();
        for &u_raw in nbrs {
            let u = graph.bitset_slot_of(u_raw);
            let ub = u * w;
            crate::meter::charge(w as u64);
            for j in 0..w {
                let mut mask = graph.bitset[vb + j] & !graph.bitset[ub + j];
                if j == vi / 64 {
                    mask &= !(1u64 << (vi % 64));
                }
                if j == u / 64 {
                    mask &= !(1u64 << (u % 64));
                }
                while mask != 0 {
                    let other = graph.bitset_vertex_at(j * 64 + mask.trailing_zeros() as usize);
                    mask &= mask - 1;
                    if u_raw < other {
                        if pacer.due() && crate::deadline::expired(deadline) {
                            return false;
                        }
                        self.fill_edge_bs(graph, vi, u_raw, other);
                    }
                }
            }
        }
        true
    }

    /// One fill edge (x, y) of v, both in N(v): its common neighbours
    /// inside N(v) each lose one missing pair from A ∩ B, those outside lose
    /// one from their own fill, and their number is what x and y each gain
    /// in e(A \ B, B \ A).
    #[inline]
    fn fill_edge_bs(&mut self, graph: &EliminationGraph, vi: usize, x: u32, y: u32) {
        let w = graph.bitset_words;
        crate::meter::charge(w as u64);
        let xs = graph.bitset_slot_of(x) * w;
        let ys = graph.bitset_slot_of(y) * w;
        let mut out = 0u64;
        let x_row = &graph.bitset[xs..xs + w];
        let y_row = &graph.bitset[ys..ys + w];
        for (word, (&x_word, &y_word)) in x_row.iter().zip(y_row).enumerate() {
            let common = x_word & y_word;
            let inside_word = self.inside[word];
            let mut ins = common & inside_word;
            while ins != 0 {
                let slot = word * 64 + ins.trailing_zeros() as usize;
                // SAFETY: a set bit of a row is a slot the bitset was built
                // with, and the counters have one entry per vertex of the
                // graph, which is at least one per slot.
                unsafe { *self.inside_fill.get_unchecked_mut(slot) += 1 };
                ins &= ins - 1;
            }
            let mut outs = common & !inside_word;
            if word == vi / 64 {
                outs &= !(1u64 << (vi % 64));
            }
            while outs != 0 {
                let slot = word * 64 + outs.trailing_zeros() as usize;
                self.increment(slot as u32);
                out += 1;
                outs &= outs - 1;
            }
        }
        self.cross[x as usize] += out;
        self.cross[y as usize] += out;
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
        let mut pacer = DeadlinePacer::new();
        for &u in nbrs {
            // Stamp u's row: it says which of v's other neighbours u lacks,
            // which are its fill edges, and stays valid while they are
            // processed since nothing below stamps again.
            self.bump_stamp();
            let stamp = self.stamp;
            let row = &graph.adj[u as usize];
            crate::meter::charge((row.len() + k) as u64);
            let mut kept = 0u32;
            for &z in row {
                self.marker[z as usize] = stamp;
                if z != v && !self.is_inside(z) {
                    kept += 1;
                }
            }
            self.kept[u as usize] = kept;
            self.gained[u as usize] = (k as u32 + kept) - row.len() as u32;
            if !fill {
                continue;
            }
            // The fill edges in the order the elimination would record them:
            // by neighbour, then by the neighbour list, each pair once from
            // its smaller vertex.
            for &y in nbrs {
                if y != u && u < y && self.marker[y as usize] != stamp {
                    if pacer.due() && crate::deadline::expired(deadline) {
                        return false;
                    }
                    self.fill_edge_marker(graph, v, u, y, stamp);
                }
            }
        }
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
            if self.marker[z as usize] != stamp {
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
        let inside_fill = std::mem::take(&mut self.inside_fill[Self::bit_index(graph, u)]);
        let u = u as usize;
        let kept = u64::from(std::mem::take(&mut self.kept[u]));
        let gained = u64::from(std::mem::take(&mut self.gained[u]));
        let cross = std::mem::take(&mut self.cross[u]);
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

/// Priority → vertex buckets with O(log n) insert/remove and O(1) indexed
/// access into the min-key bucket. The min bucket *is* the tie set (no
/// secondary key), so a caller can sample from it directly — mirrors htd's
/// `PriorityQueue::topCollection`.
#[derive(Clone)]
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

const MAX_DENSE_PRIORITY_KEY: usize = (1 << 16) - 1;

#[derive(Clone)]
enum PriorityBuckets {
    Dense {
        slots: Vec<Option<Bucket>>,
        max_key: usize,
    },
    Hashed(PriorityHashMap),
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

#[derive(Clone)]
pub(super) struct BucketMap<'a> {
    buckets: PriorityBuckets,
    /// Exact while `minimum_dirty` is false. Removing the current minimum
    /// marks it dirty; the next read scans the live priority keys once.
    minimum_key: Option<u64>,
    minimum_dirty: bool,
    spare_vertices: Vec<Vec<u32>>,
    position: Vec<BucketPosition>,
    weights: &'a [u32],
    uniform_mass: Option<u64>,
}

impl<'a> BucketMap<'a> {
    fn with_weights(weights: &'a [u32], uniform_mass: Option<u64>) -> Self {
        // Most fill-based priorities stay close to the vertex count. Direct
        // slots avoid hashing there; a cap bounds their allocation before a
        // large score switches the map to hashed storage.
        let max_dense_key = weights
            .len()
            .saturating_mul(4)
            .clamp(256, MAX_DENSE_PRIORITY_KEY);
        BucketMap {
            buckets: PriorityBuckets::Dense {
                slots: Vec::new(),
                max_key: max_dense_key,
            },
            minimum_key: None,
            minimum_dirty: false,
            spare_vertices: Vec::new(),
            position: vec![BucketPosition::VACANT; weights.len()],
            weights,
            uniform_mass,
        }
    }

    fn insert(&mut self, v: u32, key: u64) {
        let dense_key = usize::try_from(key).ok();
        let promote = matches!(
            &self.buckets,
            PriorityBuckets::Dense { max_key, .. }
                if dense_key.is_none_or(|key| key > *max_key)
        );
        if promote {
            self.promote_buckets_to_hash();
        }

        let bucket = match &mut self.buckets {
            PriorityBuckets::Dense { slots, .. } => {
                let key_index = dense_key.expect("dense priority key");
                if key_index >= slots.len() {
                    slots.resize_with(key_index + 1, || None);
                }
                if slots[key_index].is_none() {
                    self.minimum_key =
                        Some(self.minimum_key.map_or(key, |minimum| minimum.min(key)));
                    slots[key_index] = Some(Bucket {
                        vertices: self.spare_vertices.pop().unwrap_or_default(),
                        sampling_mass: 0,
                    });
                }
                slots[key_index].as_mut().expect("inserted priority bucket")
            }
            PriorityBuckets::Hashed(buckets) => match buckets.entry(key) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    self.minimum_key =
                        Some(self.minimum_key.map_or(key, |minimum| minimum.min(key)));
                    entry.insert(Bucket {
                        vertices: self.spare_vertices.pop().unwrap_or_default(),
                        sampling_mass: 0,
                    })
                }
            },
        };
        let idx = bucket.vertices.len();
        bucket.vertices.push(v);
        if self.uniform_mass.is_none() {
            bucket.sampling_mass += sampling_mass(self.weights[v as usize]);
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

    #[inline]
    fn remove_at(&mut self, v: u32, position: BucketPosition) {
        match &mut self.buckets {
            PriorityBuckets::Dense { slots, .. } => {
                let key = usize::try_from(position.key).expect("dense priority key");
                let bucket = slots[key].as_mut().expect("bucket missing");
                if Self::remove_from_bucket(
                    bucket,
                    &mut self.position,
                    self.weights,
                    self.uniform_mass,
                    v,
                    position,
                ) {
                    let bucket = slots[key].take().expect("bucket missing");
                    self.spare_vertices.push(bucket.vertices);
                    self.minimum_dirty |= self.minimum_key == Some(position.key);
                }
            }
            PriorityBuckets::Hashed(buckets) => {
                let mut entry = match buckets.entry(position.key) {
                    Entry::Occupied(entry) => entry,
                    Entry::Vacant(_) => panic!("bucket missing"),
                };
                let bucket = entry.get_mut();
                if Self::remove_from_bucket(
                    bucket,
                    &mut self.position,
                    self.weights,
                    self.uniform_mass,
                    v,
                    position,
                ) {
                    let (_, bucket) = entry.remove_entry();
                    self.spare_vertices.push(bucket.vertices);
                    self.minimum_dirty |= self.minimum_key == Some(position.key);
                }
            }
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
            self.minimum_key = match &self.buckets {
                PriorityBuckets::Dense { slots, .. } => {
                    let start = self
                        .minimum_key
                        .and_then(|key| usize::try_from(key).ok())
                        .unwrap_or(0)
                        .min(slots.len());
                    slots[start..]
                        .iter()
                        .position(Option::is_some)
                        .map(|offset| (start + offset) as u64)
                }
                PriorityBuckets::Hashed(buckets) => buckets.keys().copied().min(),
            };
            self.minimum_dirty = false;
        }
        self.minimum_key
    }

    /// The tie set a sampler draws from: every vertex whose key is within
    /// `band` of the minimum, and their combined sampling mass.
    ///
    /// A `band` of 0 hands back the minimum bucket's own slice, so the draw
    /// sees the same tie set in the same order it always has and `scratch` is
    /// left alone. A wider band copies the buckets from the minimum upwards
    /// into `scratch`, ascending by key and each bucket in storage order, so
    /// the tie set is a function of the map's history and not of the copy.
    fn min_band<'b>(
        &'b mut self,
        band: u64,
        scratch: &'b mut Vec<u32>,
    ) -> Option<(&'b [u32], u64)> {
        let minimum = self.minimum()?;
        if band == 0 {
            let bucket = self.bucket(minimum).expect("minimum bucket missing");
            return Some((bucket.vertices.as_slice(), self.bucket_mass(bucket)));
        }
        scratch.clear();
        let mut mass = 0;
        let top = minimum.saturating_add(band);
        let mut key = minimum;
        loop {
            if let Some(bucket) = self.bucket(key) {
                scratch.extend_from_slice(&bucket.vertices);
                mass += self.bucket_mass(bucket);
            }
            if key == top {
                break;
            }
            key += 1;
        }
        Some((scratch.as_slice(), mass))
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
        match &self.buckets {
            PriorityBuckets::Dense { slots, .. } => usize::try_from(key)
                .ok()
                .and_then(|key| slots.get(key))
                .and_then(Option::as_ref),
            PriorityBuckets::Hashed(buckets) => buckets.get(&key),
        }
    }

    fn promote_buckets_to_hash(&mut self) {
        let PriorityBuckets::Dense { slots, .. } = std::mem::replace(
            &mut self.buckets,
            PriorityBuckets::Hashed(PriorityHashMap::default()),
        ) else {
            return;
        };
        let PriorityBuckets::Hashed(buckets) = &mut self.buckets else {
            unreachable!();
        };
        for (key, bucket) in slots.into_iter().enumerate() {
            if let Some(bucket) = bucket {
                buckets.insert(key as u64, bucket);
            }
        }
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

/// Pick one vertex from `tie_set`, giving smaller weights more mass.
/// A one-vertex tie set draws nothing at all, so the RNG stream depends only
/// on the ties the elimination actually had to break.
fn sample_tie_set(
    tie_set: &[u32],
    weights: &[u32],
    rng: &mut Xorshift64,
    uniform_mass: Option<u64>,
    total_mass: u64,
) -> u32 {
    debug_assert!(!tie_set.is_empty());
    if tie_set.len() == 1 {
        return tie_set[0];
    }
    // Compose two u32 draws into one u64 so the draw covers `total_mass` up to
    // 2^64.
    let hi = rng.next_u32() as u64;
    let lo = rng.next_u32() as u64;
    let r = ((hi << 32) | lo) % total_mass;
    if let Some(mass) = uniform_mass {
        let pick = (r / mass) as usize;
        crate::meter::charge(1);
        return tie_set[pick];
    }
    let mut acc: u64 = 0;
    // The chosen index is tracked rather than returned from inside the loop, so
    // that the scan below can be charged on the way out. The initial value is
    // the last element, the same fall-through the walk had before: `r` is
    // reduced modulo `total`, so the final partial sum always covers it and the
    // loop is expected to break.
    let mut pick = tie_set.len() - 1;
    for (i, &v) in tie_set.iter().enumerate() {
        acc += sampling_mass(weights[v as usize]);
        if r < acc {
            pick = i;
            break;
        }
    }
    // Bucket membership updates maintain the total mass. The remaining prefix
    // walk is the dominant cost for nonuniform weights, so charge its touches
    // at the same rate as graph touches.
    crate::meter::charge(pick as u64 + 1);
    tie_set[pick]
}
