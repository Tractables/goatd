//! Shared bookkeeping for graph and hypergraph bisection.
//!
//! Their cut objectives and gain formulas differ. Balance bounds, projection
//! between levels, move rollback, gain queues, and stall detection do not, so
//! those operations live here without depending on either representation.
//!
//! # Where the two bisectors differ, and why
//!
//! - **What the caller asked for.** The graph bisector's consumers want a
//!   small separator; the hypergraph bisector's want a small hyperedge cut.
//! - **One sweep against best-of-N.** The graph side takes a single
//!   well-refined pass. Minimum edge cut does not correlate with minimum
//!   separator width, so it does not rank whole restarts by edge cut. The
//!   hypergraph side does rank restarts by its stated cut objective. Its
//!   restart and V-cycle counts both scale with the square root of effort.
//! - **Restarts of the initial partition.** Fixed at 4 on the graph side; 6
//!   above 30 vertices and 4 below on the hypergraph side.
//! - **The greedy-growing gain.** The graph side updates a score incrementally
//!   that ranks candidates but is not the cut reduction; the hypergraph side
//!   recomputes the exact gain each step, paying a scan of every unplaced
//!   vertex's incidences for it.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::Error;
use crate::rng::Xorshift64;

/// Check the balance tolerance shared by both public bisectors.
pub(super) fn validate_max_imbalance(value: f64, kind: &str) -> Result<(), Error> {
    if !value.is_finite() || !(0.0..=0.5).contains(&value) {
        return Err(Error::InvalidInput(format!(
            "{kind} imbalance must be in 0.0..=0.5, got {value}"
        )));
    }
    Ok(())
}

/// The bisection of `n` vertices by index: the first half to side 0, the rest
/// to side 1.
///
/// The answer both bisectors fall back on when the partitioner has nothing to
/// work with — no edges at all, or a pass that put every vertex on one side.
/// Neither is a partition a caller can recurse into, and a caller that asked
/// for a bisection has to get two non-empty sides for any `n >= 2`.
pub(super) fn index_split(n: usize) -> Vec<u8> {
    let mut part = vec![0u8; n];
    part[n / 2..].fill(1);
    part
}

/// The bisection of `n` vertices that needs no partitioner at all, for the
/// three sizes where there is only one answer; `None` once there is a choice
/// to make.
pub(super) fn tiny_bisection(n: usize) -> Option<Vec<u8>> {
    match n {
        0 => Some(Vec::new()),
        1 => Some(vec![0]),
        2 => Some(vec![0, 1]),
        _ => None,
    }
}

/// Project a fine bisection onto coarse vertices by majority vote, with ties
/// assigned to side 0.
pub(super) fn project_to_coarse(
    fine: &[u8],
    mapping: &[u32],
    num_coarse_vertices: usize,
    counts: &mut Vec<[u32; 2]>,
    coarse: &mut Vec<u8>,
) {
    counts.clear();
    counts.resize(num_coarse_vertices, [0, 0]);
    for (vertex, &coarse_vertex) in mapping.iter().enumerate() {
        counts[coarse_vertex as usize][fine[vertex] as usize] += 1;
    }

    coarse.clear();
    coarse.resize(num_coarse_vertices, 0);
    for vertex in 0..num_coarse_vertices {
        coarse[vertex] = u8::from(counts[vertex][1] > counts[vertex][0]);
    }
}

/// Lift a coarse bisection to the fine vertices represented by `mapping`.
pub(super) fn lift_to_fine(coarse: &[u8], mapping: &[u32], fine: &mut Vec<u8>) {
    fine.clear();
    fine.extend(mapping.iter().map(|&vertex| coarse[vertex as usize]));
}

/// Enforce the public bisection contract after uncoarsening. At the finest
/// level every vertex has unit weight, so moving the last assignments from an
/// oversized side repairs the count while changing as few vertices as
/// possible. For two or more vertices, each side retains at least one.
pub(super) fn repair_bisection(mut part: Vec<u8>, max_imbalance: f64) -> Vec<u8> {
    let num_vertices = part.len();
    if num_vertices < 2 {
        return part;
    }

    let max_side_size = ((num_vertices as f64) * (0.5 + max_imbalance))
        .ceil()
        .min((num_vertices - 1) as f64) as usize;
    let side_zero_size = part.iter().filter(|&&side| side == 0).count();
    if side_zero_size > max_side_size {
        let mut to_move = side_zero_size - max_side_size;
        for side in part.iter_mut().rev() {
            if *side == 0 && to_move > 0 {
                *side = 1;
                to_move -= 1;
            }
        }
    } else if num_vertices - side_zero_size > max_side_size {
        let mut to_move = num_vertices - side_zero_size - max_side_size;
        for side in part.iter_mut().rev() {
            if *side == 1 && to_move > 0 {
                *side = 0;
                to_move -= 1;
            }
        }
    }
    part
}

/// The weight window either side of a bisection has to stay in under
/// `max_imbalance`, as `(min, max)`.
///
/// One bound, applied to both sides: whatever `max_imbalance` allows the heavy
/// side is denied to the light one. Weights are in fine-vertex units, so at coarse
/// levels a single vertex can be too heavy to move anywhere.
pub(super) fn balance_bounds(vertex_weights: &[u32], max_imbalance: f64) -> (u32, u32) {
    let total_weight: u32 = vertex_weights.iter().sum();
    let max_part_weight = ((total_weight as f64) * (0.5 + max_imbalance)).ceil() as u32;
    let min_part_weight = total_weight.saturating_sub(max_part_weight);
    (min_part_weight, max_part_weight)
}

/// The balance a Fiduccia-Mattheyses pass starts from: the weight already on
/// each side of the partition, and the window a move has to leave both sides
/// inside.
pub(super) struct FmBalance {
    /// Total vertex weight on each side.
    pub(super) weight: [u32; 2],
    /// The lightest a side may become.
    pub(super) min_part_weight: u32,
    /// The heaviest a side may become.
    pub(super) max_part_weight: u32,
}

/// Weigh `part` and read off the balance window, or `None` when there is
/// nothing for a pass to do — with two vertices or fewer, moving one cannot
/// improve a cut without emptying a side.
///
/// `n` is the vertex count of the graph `part` partitions; `vertex_weights` is that
/// graph's vertex weights.
pub(super) fn fm_balance(
    n: usize,
    vertex_weights: &[u32],
    part: &[u8],
    max_imbalance: f64,
) -> Option<FmBalance> {
    if n <= 2 {
        return None;
    }
    let (min_part_weight, max_part_weight) = balance_bounds(vertex_weights, max_imbalance);
    let mut weight = [0u32; 2];
    for v in 0..n {
        weight[part[v] as usize] += vertex_weights[v];
    }
    Some(FmBalance {
        weight,
        min_part_weight,
        max_part_weight,
    })
}

/// Fills side 0 in random order while the next vertex still fits under half the
/// total weight.
///
/// A vertex that does not fit is skipped and never reconsidered, so at coarse
/// levels — where vertex weights are large and uneven — side 0 can finish well
/// short of half. FM is what pulls the result back toward balance.
pub(super) fn random_bisection(vertex_weights: &[u32], rng: &mut Xorshift64) -> Vec<u8> {
    let n = vertex_weights.len();
    let total_weight: u32 = vertex_weights.iter().sum();
    let target = total_weight / 2;

    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = (rng.next_u64() as usize) % (i + 1);
        perm.swap(i, j);
    }

    let mut part = vec![1u8; n];
    let mut weight0: u32 = 0;
    for &v in &perm {
        if weight0 + vertex_weights[v] <= target {
            part[v] = 0;
            weight0 += vertex_weights[v];
        }
    }
    part
}

/// Keep the best prefix of a finished move sequence and undo everything after
/// it, reporting whether anything survived.
///
/// `moves` is the sequence in the order it was applied to `part`, and
/// `cumulative_gain[i]` is the running gain after move `i`. A strictly positive
/// best prefix gain is required, so a pass that only matched the starting cut
/// reports no improvement and unwinds completely rather than handing back an
/// equal-cut partition the caller would loop on. On `false` — an empty sequence
/// included — `part` comes back exactly as the pass found it.
pub(super) fn commit_best_prefix(
    moves: &[usize],
    cumulative_gain: &[i64],
    part: &mut [u8],
) -> bool {
    if moves.is_empty() {
        return false;
    }

    let mut best_index = None;
    let mut best_prefix_gain = 0i64;
    for (index, &gain) in cumulative_gain.iter().enumerate() {
        if gain > best_prefix_gain {
            best_prefix_gain = gain;
            best_index = Some(index);
        }
    }

    let Some(best_index) = best_index else {
        for &v in moves.iter().rev() {
            part[v] = 1 - part[v];
        }
        return false;
    };

    for &v in moves[(best_index + 1)..].iter().rev() {
        part[v] = 1 - part[v];
    }

    true
}

/// Vertices bucketed by gain. Each bucket is a stack, so the most recently
/// inserted or updated vertex wins a gain tie.
pub(super) struct GainBuckets {
    buckets: BTreeMap<i64, Vec<usize>>,
    /// The gain whose bucket holds `v`, or `None` when it is not queued.
    gain_of: Vec<Option<i64>>,
    /// Position of `v` within its bucket while it is queued.
    pos_in_bucket: Vec<usize>,
}

impl GainBuckets {
    /// An empty queue over `n` vertices.
    pub(super) fn new(n: usize) -> Self {
        let mut queue = GainBuckets::empty();
        queue.reset(n);
        queue
    }

    /// A queue with no vertex slots, for reuse across passes.
    pub(super) fn empty() -> Self {
        GainBuckets {
            buckets: BTreeMap::new(),
            gain_of: Vec::new(),
            pos_in_bucket: Vec::new(),
        }
    }

    /// Empty the queue and resize its per-vertex index.
    pub(super) fn reset(&mut self, n: usize) {
        self.buckets.clear();
        self.gain_of.clear();
        self.gain_of.resize(n, None);
        self.pos_in_bucket.clear();
        self.pos_in_bucket.resize(n, usize::MAX);
    }

    /// Is `v` queued?
    pub(super) fn contains(&self, v: usize) -> bool {
        self.gain_of[v].is_some()
    }

    /// The best-gain vertex accepted by `predicate`, left in the queue.
    /// Gain buckets and their vertices are both visited from their preferred
    /// end, preserving the queue's gain and recency ordering while allowing a
    /// temporarily balance-infeasible vertex to remain queued.
    pub(super) fn best_satisfying(
        &self,
        mut predicate: impl FnMut(usize) -> bool,
    ) -> Option<usize> {
        self.buckets
            .iter()
            .rev()
            .flat_map(|(_, vertices)| vertices.iter().rev().copied())
            .find(|&vertex| predicate(vertex))
    }

    /// Queue `v` at `gain`. Appending is what makes the tie-break the most
    /// recent vertex rather than an arbitrary one.
    pub(super) fn insert(&mut self, v: usize, gain: i64) {
        debug_assert!(self.gain_of[v].is_none());
        let bucket = self.buckets.entry(gain).or_default();
        self.pos_in_bucket[v] = bucket.len();
        bucket.push(v);
        self.gain_of[v] = Some(gain);
    }

    /// Take `v` out of the queue; a no-op for a vertex that is not in it.
    pub(super) fn remove(&mut self, v: usize) {
        let Some(gain) = self.gain_of[v].take() else {
            return;
        };
        let pos = self.pos_in_bucket[v];
        let remove_bucket = {
            let bucket = self.buckets.get_mut(&gain).expect("gain bucket missing");
            bucket.swap_remove(pos);
            if pos < bucket.len() {
                let moved = bucket[pos];
                self.pos_in_bucket[moved] = pos;
            }
            bucket.is_empty()
        };
        if remove_bucket {
            self.buckets.remove(&gain);
        }
        self.pos_in_bucket[v] = usize::MAX;
    }

    /// Re-file `v` under `new_gain`, which also puts it at the head of the
    /// tie-break among its new equals.
    pub(super) fn update(&mut self, v: usize, new_gain: i64) {
        self.remove(v);
        self.insert(v, new_gain);
    }
}

/// How long a pass has gone without bettering the best running gain it has
/// seen, and how long it is allowed to.
///
/// Both refiners stop short of the textbook pass, which moves every vertex
/// before rolling back to the best prefix. Losing moves can escape a local
/// minimum, but a long non-improving suffix is likely to be rolled back.
pub(super) struct Stall {
    limit: usize,
    since_improvement: usize,
    best_gain: i64,
}

impl Stall {
    /// `limit` consecutive moves without an improvement end the pass.
    pub(super) fn new(limit: usize) -> Self {
        Stall {
            limit,
            since_improvement: 0,
            best_gain: 0,
        }
    }

    /// Record the running gain after a move, reporting whether the pass has
    /// stalled.
    pub(super) fn record(&mut self, running_gain: i64) -> bool {
        if running_gain > self.best_gain {
            self.best_gain = running_gain;
            self.since_improvement = 0;
            false
        } else {
            self.since_improvement += 1;
            self.since_improvement >= self.limit
        }
    }
}
