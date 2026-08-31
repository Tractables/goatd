//! Greedy min-fill and min-degree elimination cores.
//!
//! The deterministic cores use a seeded salt after their algorithmic keys.
//! The sampling cores draw from the full set tied on the primary key.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::time::Instant;

use super::execution::{DEADLINE_CHECK_STRIDE, ElimExit, ElimSink, ElimStop, exceeds_width_bound};
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

mod deterministic;
mod min_degree;
mod min_fill;
mod sampling;

#[cfg(test)]
mod tests;

pub(super) use min_degree::eliminate_min_degree;
pub(super) use min_fill::eliminate_min_fill;
pub(super) use sampling::{eliminate_sampled_min_degree, eliminate_sampled_min_fill};

/// Above this many active vertices, a single cheap-mode eliminate on a dense
/// residual can overshoot the deadline by seconds. Emergency-bail immediately
/// on deadline rather than attempting cheap-mode elimination at this scale.
pub(super) const CHEAP_MODE_MAX_ACTIVE: usize = 512;

/// Scratch state for min-fill's fill-count computation, reused across calls
/// to avoid reallocating per vertex.
struct FillScratch {
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
    fn new(n: usize) -> Self {
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
    fn fill_count_of(&mut self, graph: &EliminationGraph, v: u32) -> u64 {
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
        // is one of the two places a min-fill candidate spends its budget, the other
        // being `EliminationGraph::eliminate_with_nbrs`. The summation is guarded because
        // computing it is itself a pass over the same rows, and off the meter
        // nothing would read the result.
        if crate::meter::is_armed() {
            let sigma: u64 = nbrs_v
                .iter()
                .map(|&u| graph.adj[u as usize].len() as u64)
                .sum();
            crate::meter::charge(k as u64 + sigma);
        }
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

/// Vertices whose fill key may change after eliminating one vertex.
///
/// Immediate neighbours are re-scored separately because their neighbourhood
/// loses the eliminated vertex. For each fill edge, this tracker finds active
/// common neighbours outside the filled neighbourhood. Each such edge lowers
/// their fill score by exactly one.
struct FillAffected {
    inside: Vec<bool>,
    marker: Vec<u16>,
    stamp: u16,
    delta: Vec<u64>,
    vertices: Vec<u32>,
}

impl FillAffected {
    fn new(n: usize) -> Self {
        Self {
            inside: vec![false; n],
            marker: vec![0; n],
            stamp: 0,
            delta: vec![0; n],
            vertices: Vec::new(),
        }
    }

    fn clear(&mut self) {
        for &vertex in &self.vertices {
            self.delta[vertex as usize] = 0;
        }
        self.vertices.clear();
    }

    #[inline]
    fn bump_stamp(&mut self) {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.marker.fill(0);
            self.stamp = 1;
        }
    }

    fn increment(&mut self, vertex: u32) {
        let index = vertex as usize;
        if self.delta[index] == 0 {
            self.vertices.push(vertex);
        }
        self.delta[index] += 1;
    }

    /// Accumulate exact fill-score decreases caused by `fill_edges`. Returns
    /// false after clearing its scratch if `deadline` passes during the scan.
    fn collect_deltas(
        &mut self,
        graph: &EliminationGraph,
        nbrs: &[u32],
        fill_edges: &[(u32, u32)],
        deadline: Option<Instant>,
    ) -> bool {
        debug_assert!(self.vertices.is_empty());
        for &vertex in nbrs {
            self.inside[vertex as usize] = true;
        }

        for &(left, right) in fill_edges {
            if crate::deadline::expired(deadline) {
                self.clear();
                for &vertex in nbrs {
                    self.inside[vertex as usize] = false;
                }
                return false;
            }
            if graph.bitset_words > 0 {
                let words = graph.bitset_words;
                crate::meter::charge(words as u64);
                let left_start = left as usize * words;
                let right_start = right as usize * words;
                for word in 0..words {
                    let mut common =
                        graph.bitset[left_start + word] & graph.bitset[right_start + word];
                    while common != 0 {
                        let bit = common.trailing_zeros() as usize;
                        let vertex = (word * 64 + bit) as u32;
                        if !self.inside[vertex as usize] {
                            self.increment(vertex);
                        }
                        common &= common - 1;
                    }
                }
            } else {
                self.bump_stamp();
                let stamp = self.stamp;
                let left_row = &graph.adj[left as usize];
                let right_row = &graph.adj[right as usize];
                crate::meter::charge((left_row.len() + right_row.len()) as u64);
                for &vertex in left_row {
                    self.marker[vertex as usize] = stamp;
                }
                for &vertex in right_row {
                    if self.marker[vertex as usize] == stamp && !self.inside[vertex as usize] {
                        self.increment(vertex);
                    }
                }
            }
        }
        for &vertex in nbrs {
            self.inside[vertex as usize] = false;
        }
        true
    }

    fn pop_delta(&mut self) -> Option<(u32, u64)> {
        self.vertices.pop().map(|vertex| {
            let delta = std::mem::replace(&mut self.delta[vertex as usize], 0);
            (vertex, delta)
        })
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
pub(super) struct BucketMap {
    buckets: BTreeMap<u64, Vec<u32>>,
    position: Vec<Option<(u64, usize)>>,
}

impl BucketMap {
    fn with_capacity(n: usize) -> Self {
        BucketMap {
            buckets: BTreeMap::new(),
            position: vec![None; n],
        }
    }

    fn insert(&mut self, v: u32, key: u64) {
        let bucket = self.buckets.entry(key).or_default();
        let idx = bucket.len();
        bucket.push(v);
        self.position[v as usize] = Some((key, idx));
    }

    fn remove_vertex(&mut self, v: u32) {
        if let Some((key, idx)) = self.position[v as usize].take() {
            let bucket = self.buckets.get_mut(&key).expect("bucket missing");
            let last_idx = bucket.len() - 1;
            if idx != last_idx {
                let moved = bucket[last_idx];
                bucket[idx] = moved;
                self.position[moved as usize] = Some((key, idx));
            }
            bucket.pop();
            if bucket.is_empty() {
                self.buckets.remove(&key);
            }
        }
    }

    fn update(&mut self, v: u32, new_key: u64) {
        if let Some((cur_key, _)) = self.position[v as usize] {
            if cur_key == new_key {
                return;
            }
            self.remove_vertex(v);
        }
        self.insert(v, new_key);
    }

    fn min_bucket(&self) -> Option<(u64, &[u32])> {
        self.buckets
            .iter()
            .next()
            .map(|(key, vertices)| (*key, vertices.as_slice()))
    }

    fn key_of(&self, v: u32) -> Option<u64> {
        self.position[v as usize].map(|(key, _)| key)
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

/// Pick one vertex from `tie_set`, giving smaller weights more mass.
/// A one-vertex tie set draws nothing at all, so the RNG stream depends only
/// on the ties the elimination actually had to break.
fn sample_tie_set(tie_set: &[u32], weights: &[u32], rng: &mut Xorshift64) -> u32 {
    debug_assert!(!tie_set.is_empty());
    if tie_set.len() == 1 {
        return tie_set[0];
    }
    let mut total: u64 = 0;
    for &v in tie_set {
        total += sampling_mass(weights[v as usize]);
    }
    // Compose two u32 draws into one u64 so the draw covers `total` up to 2^64.
    let hi = rng.next_u32() as u64;
    let lo = rng.next_u32() as u64;
    let r = ((hi << 32) | lo) % total;
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
    // The dominant cost of weighted min-degree and min-fill sampling. Some
    // residuals put thousands of same-degree vertices in one bucket, so the two
    // passes over `tie_set` above outweigh the graph mutation that follows them
    // by more than an order of magnitude: measured on one such residual, 316 M
    // tie-set touches over an elimination run against 12 M units of charged
    // graph work. A touch here costs what a charged graph touch costs, so the
    // scan goes on the meter at face value and needs no weight of its own.
    crate::meter::charge(tie_set.len() as u64 + pick as u64 + 1);
    tie_set[pick]
}
