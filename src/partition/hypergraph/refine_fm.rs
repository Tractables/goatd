//! Fiduccia-Mattheyses refinement of an existing hypergraph partition: bucket-queue global
//! passes at every level, plus localized passes seeded on the boundary at the
//! finest one.
//!
//! A pass moves vertices one at a time and never moves the same vertex twice,
//! then keeps the best *prefix* of that move sequence and rolls the rest back.
//! Negative-gain moves are allowed, but survive rollback only if later moves
//! make their prefix profitable. This lets FM leave a single-move local
//! minimum.
//!
//! Hypergraph gains hinge on two pin counts per hyperedge rather than on a
//! single edge's endpoints, so `pin_counts` is maintained alongside the partition
//! and every rule below is a statement about a count reaching 0, 1, or 2.

use std::collections::VecDeque;

use super::model::Hypergraph;
use crate::partition::common::{
    FmBalance, GainBuckets, Stall, commit_best_prefix, fm_balance, select_move,
};

pub(super) struct FmScratch {
    gain: Vec<i64>,
    locked: Vec<bool>,
    moves: Vec<usize>,
    cumulative_gain: Vec<i64>,
    bq: [GainBuckets; 2],
    pub(super) region: RegionScratch,
}

impl FmScratch {
    pub(super) fn new() -> Self {
        FmScratch {
            gain: Vec::new(),
            locked: Vec::new(),
            moves: Vec::new(),
            cumulative_gain: Vec::new(),
            bq: [GainBuckets::empty(), GainBuckets::empty()],
            region: RegionScratch::new(),
        }
    }

    fn prepare(&mut self, n: usize) {
        self.gain.clear();
        self.gain.resize(n, 0);
        self.locked.clear();
        self.locked.resize(n, false);
        self.moves.clear();
        self.cumulative_gain.clear();
        self.bq[0].reset(n);
        self.bq[1].reset(n);
    }
}

/// Working storage for [`localized_fm_pass`], held across the four tries of a
/// level and across the levels of a sweep.
///
/// A localized pass touches at most `max_region` vertices out of `n`, so the
/// per-vertex arrays are cleared over the region the pass built rather than over
/// the whole hypergraph. That leaves every array as the pass found it, which is
/// what lets `prepare` be a resize that usually does nothing.
pub(super) struct RegionScratch {
    gain: Vec<i64>,
    in_region: Vec<bool>,
    locked: Vec<bool>,
    region_list: Vec<usize>,
    queue: VecDeque<usize>,
    moves: Vec<usize>,
    cumulative_gain: Vec<i64>,
}

impl RegionScratch {
    pub(super) fn new() -> Self {
        RegionScratch {
            gain: Vec::new(),
            in_region: Vec::new(),
            locked: Vec::new(),
            region_list: Vec::new(),
            queue: VecDeque::new(),
            moves: Vec::new(),
            cumulative_gain: Vec::new(),
        }
    }

    /// Size the per-vertex arrays for a hypergraph of `n` vertices and empty the
    /// lists. The arrays keep the values they hold, which the previous pass left
    /// cleared; a length that grows is filled with the same cleared value.
    fn prepare(&mut self, n: usize) {
        self.gain.resize(n, 0);
        self.in_region.resize(n, false);
        self.locked.resize(n, false);
        // The pass that ends leaves these as it found them. `gain` is exempt:
        // every region vertex is written before it is read, and no other entry
        // is read at all.
        debug_assert!(self.in_region.iter().all(|&member| !member));
        debug_assert!(self.locked.iter().all(|&locked| !locked));
        self.region_list.clear();
        self.queue.clear();
        self.moves.clear();
        self.cumulative_gain.clear();
    }
}

pub(super) fn fm_refine_pass(
    hg: &Hypergraph,
    part: &mut [u8],
    max_imbalance: f64,
    scratch: &mut FmScratch,
) -> bool {
    let n = hg.num_vertices;
    let Some(mut balance) = fm_balance(n, &hg.vertex_weights, part, max_imbalance) else {
        return false;
    };

    let mut pin_counts = hg.pin_counts(part);

    scratch.prepare(n);
    let gain = scratch.gain.as_mut_slice();
    let bq = &mut scratch.bq;

    for v in 0..n {
        let from = part[v] as usize;
        let to = 1 - from;
        // Gain of moving `v`: a hyperedge it is the last pin of on its own side
        // stops being cut (+w), and one with no pin yet on the far side starts
        // being cut (-w). A hyperedge with company on both sides is cut either
        // way and contributes nothing. Only vertices on a cut hyperedge are
        // queued — the rest can only add to the cut.
        let mut g = 0i64;
        let mut on_boundary = false;
        for &hei in hg.vertex_hyperedges(v) {
            let hei = hei as usize;
            let w = i64::from(hg.hyperedge_weights[hei]);
            if pin_counts[hei][from] == 1 {
                g += w;
            }
            if pin_counts[hei][to] == 0 {
                g -= w;
            }
            if pin_counts[hei][0] > 0 && pin_counts[hei][1] > 0 {
                on_boundary = true;
            }
        }
        gain[v] = g;
        if on_boundary {
            bq[from].insert(v, g);
        }
    }

    let locked = scratch.locked.as_mut_slice();
    let moves = &mut scratch.moves;
    let cumulative_gain = &mut scratch.cumulative_gain;
    let mut running_gain: i64 = 0;
    let mut stall = Stall::new((n / 2).max(20));

    for _ in 0..n {
        let Some((v, from, best_gain)) =
            select_move(bq, gain, locked, &hg.vertex_weights, &balance)
        else {
            break;
        };
        let to = 1 - from;

        bq[from].remove(v);
        balance.weight[from] -= hg.vertex_weights[v];
        balance.weight[to] += hg.vertex_weights[v];
        part[v] = to as u8;
        locked[v] = true;

        running_gain += best_gain;
        moves.push(v);
        cumulative_gain.push(running_gain);

        if stall.record(running_gain) {
            break;
        }

        // Incremental gain updates: O(hyperedge_size) per hyperedge instead of
        // O(hyperedge_size × avg_degree) for full recomputation.
        for &hei in hg.vertex_hyperedges(v) {
            let hei = hei as usize;
            let old_from = pin_counts[hei][from];
            let old_to = pin_counts[hei][to];

            pin_counts[hei][from] -= 1;
            pin_counts[hei][to] += 1;

            let new_from = old_from - 1;
            let _new_to = old_to + 1;

            for &u in hg.charged_hyperedge_pins(hei) {
                let u = u as usize;
                if locked[u] {
                    continue;
                }

                // Four transitions, each a pin count crossing a critical
                // value. For `u` still on the side `v` left: `old_from == 2`
                // means `u` is now the last pin holding the hyperedge on that
                // side, so moving `u` would close it (+w); `old_to == 0` means
                // `v` just opened the far side, so `u` following no longer
                // opens it (+w). For `u` on the side `v` joined: `old_to == 1`
                // means `u` was the lone pin there and no longer is (-w);
                // `new_from == 0` means `v` was the last pin on the far side,
                // so `u` going back would reopen it (-w). The mirrored tests
                // are absent because each would require a side to hold no pins
                // while `v` or `u` is sitting on it.
                let u_side = part[u] as usize;
                let w = i64::from(hg.hyperedge_weights[hei]);
                let mut delta = 0i64;

                if u_side == from {
                    if old_from == 2 {
                        delta += w;
                    }
                    if old_to == 0 {
                        delta += w;
                    }
                } else {
                    if old_to == 1 {
                        delta -= w;
                    }
                    if new_from == 0 {
                        delta -= w;
                    }
                }

                if delta != 0 {
                    gain[u] += delta;
                }

                // Queue membership is per vertex, not per hyperedge: `u` leaves
                // only when none of its hyperedges is cut any more, so the
                // rescan below runs just on the step where this hyperedge
                // changed cut state.
                let hyperedge_is_cut = pin_counts[hei][0] > 0 && pin_counts[hei][1] > 0;
                let was_in_queue = bq[u_side].contains(u);
                let was_cut = old_from > 0 && old_to > 0;

                if hyperedge_is_cut != was_cut {
                    let on_boundary = if hyperedge_is_cut {
                        true
                    } else {
                        hg.vertex_hyperedges(u).iter().any(|&hej| {
                            let hej = hej as usize;
                            pin_counts[hej][0] > 0 && pin_counts[hej][1] > 0
                        })
                    };
                    if on_boundary {
                        if was_in_queue {
                            bq[u_side].update(u, gain[u]);
                        } else {
                            bq[u_side].insert(u, gain[u]);
                        }
                    } else if was_in_queue {
                        bq[u_side].remove(u);
                    }
                } else if was_in_queue && delta != 0 {
                    bq[u_side].update(u, gain[u]);
                }
            }
        }
    }

    commit_best_prefix(moves, cumulative_gain, part)
}

/// Returns true if the partition was improved.
///
/// FM confined to a region grown around `seed`, run at the finest level after
/// the global passes have stopped improving. The region is capped, so selection
/// is a linear scan over its vertices rather than a bucket queue, and a move
/// updates only region pins; a vertex outside the region is never a candidate,
/// so its gain entry is neither written nor read. On `false` the partition is
/// restored to exactly what was passed in.
pub(super) fn localized_fm_pass(
    hg: &Hypergraph,
    part: &mut [u8],
    seed: usize,
    max_imbalance: f64,
    scratch: &mut RegionScratch,
) -> bool {
    let n = hg.num_vertices;
    let Some(FmBalance {
        mut weight,
        min_part_weight,
        max_part_weight,
    }) = fm_balance(n, &hg.vertex_weights, part, max_imbalance)
    else {
        return false;
    };

    let mut pin_counts = hg.pin_counts(part);

    scratch.prepare(n);
    let in_region = scratch.in_region.as_mut_slice();
    let region_queue = &mut scratch.queue;

    let max_region = (n / 4).max(20).min(n);
    in_region[seed] = true;
    region_queue.push_back(seed);
    let mut region_size = 1usize;

    // Growing along cut hyperedges only, and taking every pin of one, picks up
    // both sides of the cut in a single rule — a hyperedge is only cut if it
    // has pins on both. The graph sibling needs a second sweep to reach the
    // same-side vertices FM wants room to move.
    while let Some(v) = region_queue.pop_front() {
        if region_size >= max_region {
            break;
        }
        for &hei in hg.vertex_hyperedges(v) {
            let hei_idx = hei as usize;
            if pin_counts[hei_idx][0] == 0 || pin_counts[hei_idx][1] == 0 {
                continue;
            }
            for &u in hg.charged_hyperedge_pins(hei_idx) {
                let u = u as usize;
                if !in_region[u] && region_size < max_region {
                    in_region[u] = true;
                    region_queue.push_back(u);
                    region_size += 1;
                }
            }
        }
    }

    // Ascending index order (not BFS discovery order) preserves the lowest-index
    // tie-break in the move loop: `gain[v] > best_gain` keeps the first vertex
    // reaching the max.
    let region_list = &mut scratch.region_list;
    region_list.extend((0..n).filter(|&v| in_region[v]));
    debug_assert_eq!(region_list.len(), region_size);

    // Gains only for region vertices. A non-region entry is never read, so
    // `gain` is written here and left alone rather than cleared.
    let gain = scratch.gain.as_mut_slice();
    for &v in region_list.iter() {
        let from = part[v] as usize;
        let to = 1 - from;
        let mut g = 0i64;
        for &hei in hg.vertex_hyperedges(v) {
            let hei = hei as usize;
            let w = i64::from(hg.hyperedge_weights[hei]);
            if pin_counts[hei][from] == 1 {
                g += w;
            }
            if pin_counts[hei][to] == 0 {
                g -= w;
            }
        }
        gain[v] = g;
    }

    let locked = scratch.locked.as_mut_slice();
    let moves = &mut scratch.moves;
    let cumulative_gain = &mut scratch.cumulative_gain;
    let mut running_gain: i64 = 0;
    let mut stall = Stall::new(region_list.len() / 2);

    // O(region²), not O(region·n): region_list, not 0..n, is scanned per move.
    for _ in 0..region_list.len() {
        let mut best_v = None;
        let mut best_gain = i64::MIN;
        for &v in region_list.iter() {
            if locked[v] {
                continue;
            }
            let from = part[v] as usize;
            let to = 1 - from;
            let nfw = weight[from] - hg.vertex_weights[v];
            let ntw = weight[to] + hg.vertex_weights[v];
            if nfw < min_part_weight || ntw > max_part_weight {
                continue;
            }
            if best_v.is_none() || gain[v] > best_gain {
                best_gain = gain[v];
                best_v = Some(v);
            }
        }
        let Some(v) = best_v else {
            break;
        };
        let from = part[v] as usize;
        let to = 1 - from;

        weight[from] -= hg.vertex_weights[v];
        weight[to] += hg.vertex_weights[v];
        part[v] = to as u8;
        locked[v] = true;

        running_gain += best_gain;
        moves.push(v);
        cumulative_gain.push(running_gain);

        if stall.record(running_gain) {
            break;
        }

        for &hei in hg.vertex_hyperedges(v) {
            let hei = hei as usize;
            let w = i64::from(hg.hyperedge_weights[hei]);
            let old_from = pin_counts[hei][from];
            let old_to = pin_counts[hei][to];
            pin_counts[hei][from] -= 1;
            pin_counts[hei][to] += 1;
            let new_from = old_from - 1;

            // The same four transitions as `fm_refine_pass`, applied to
            // region pins only.
            for &u in hg.charged_hyperedge_pins(hei) {
                let u = u as usize;
                if locked[u] || !in_region[u] {
                    continue;
                }
                let u_side = part[u] as usize;
                let mut delta = 0i64;
                if u_side == from {
                    if old_from == 2 {
                        delta += w;
                    }
                    if old_to == 0 {
                        delta += w;
                    }
                } else {
                    if old_to == 1 {
                        delta -= w;
                    }
                    if new_from == 0 {
                        delta -= w;
                    }
                }
                if delta != 0 {
                    gain[u] += delta;
                }
            }
        }
    }

    let improved = commit_best_prefix(moves, cumulative_gain, part);

    // Hand the arrays back the way they were found. Only region vertices are
    // marked in `in_region`, and only region vertices are ever locked.
    for &v in region_list.iter() {
        in_region[v] = false;
        locked[v] = false;
    }
    improved
}

/// Standard FM refinement (global passes only).
///
/// Passes repeat until one fails to improve. The cap bounds the case where each
/// pass finds a single-move improvement and would otherwise keep going.
pub(super) fn refine_level(
    hg: &Hypergraph,
    part: &mut [u8],
    imbalance: f64,
    scratch: &mut FmScratch,
) {
    let max_passes = 10;
    for _ in 0..max_passes {
        if !fm_refine_pass(hg, part, imbalance, scratch) {
            break;
        }
    }
}
