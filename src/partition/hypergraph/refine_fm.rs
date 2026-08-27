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

use super::model::Hypergraph;
use crate::partition::common::{FmBalance, GainBuckets, Stall, commit_best_prefix, fm_balance};

pub(super) fn fm_refine_pass(hg: &Hypergraph, part: &mut [u8], max_imbalance: f64) -> bool {
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

    let mut gain = vec![0i64; n];
    let mut bq = [GainBuckets::new(n), GainBuckets::new(n)];

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

    let mut locked = vec![false; n];
    let mut moves: Vec<usize> = Vec::with_capacity(n);
    let mut cumulative_gain: Vec<i64> = Vec::with_capacity(n);
    let mut running_gain: i64 = 0;
    let mut stall = Stall::new((n / 2).max(20));

    for _ in 0..n {
        let mut best_v: Option<usize> = None;
        let mut best_gain = i64::MIN;
        let mut best_from: usize = 0;

        for side in 0..2 {
            let candidate = bq[side].best_satisfying(|vertex| {
                let to = 1 - side;
                !locked[vertex]
                    && weight[side] - hg.vertex_weights[vertex] >= min_part_weight
                    && weight[to] + hg.vertex_weights[vertex] <= max_part_weight
            });
            if let Some(vertex) = candidate {
                let g = gain[vertex];
                if g > best_gain {
                    best_gain = g;
                    best_v = Some(vertex);
                    best_from = side;
                }
            }
        }

        let v = match best_v {
            Some(v) => v,
            None => break,
        };

        let from = best_from;
        let to = 1 - from;

        bq[from].remove(v);
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

    commit_best_prefix(&moves, &cumulative_gain, part)
}

/// Returns true if the partition was improved.
///
/// FM confined to a region grown around `seed`, run at the finest level after
/// the global passes have stopped improving. The region is capped, so selection
/// is a linear scan over its vertices rather than a bucket queue, and a move
/// updates only region pins; a vertex outside the region is never a candidate,
/// so its gain entry stays at zero. On `false` the partition is restored to
/// exactly what was passed in.
pub(super) fn localized_fm_pass(
    hg: &Hypergraph,
    part: &mut [u8],
    seed: usize,
    max_imbalance: f64,
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

    let max_region = (n / 4).max(20).min(n);
    let mut in_region = vec![false; n];
    let mut region_queue = std::collections::VecDeque::new();
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

    let mut gain = vec![0i64; n];
    for v in 0..n {
        if !in_region[v] {
            continue;
        }
        let from = part[v] as usize;
        let to = 1 - from;
        for &hei in hg.vertex_hyperedges(v) {
            let hei = hei as usize;
            let w = i64::from(hg.hyperedge_weights[hei]);
            if pin_counts[hei][from] == 1 {
                gain[v] += w;
            }
            if pin_counts[hei][to] == 0 {
                gain[v] -= w;
            }
        }
    }

    let mut locked = vec![false; n];
    let mut moves: Vec<usize> = Vec::new();
    let mut cumulative_gain: Vec<i64> = Vec::new();
    let mut running_gain: i64 = 0;
    let mut stall = Stall::new(region_size / 2);

    // O(region²), not O(region·n): region_list, not 0..n, is scanned per move.
    // Ascending index order (not BFS discovery order) preserves the lowest-index
    // tie-break: `gain[v] > best_gain` keeps the first vertex reaching the max.
    let region_list: Vec<usize> = (0..n).filter(|&v| in_region[v]).collect();

    for _ in 0..region_size {
        let mut best_v = None;
        let mut best_gain = i64::MIN;
        for &v in &region_list {
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

    commit_best_prefix(&moves, &cumulative_gain, part)
}

/// Standard FM refinement (global passes only).
///
/// Passes repeat until one fails to improve. The cap bounds the case where each
/// pass finds a single-move improvement and would otherwise keep going.
pub(super) fn refine_level(hg: &Hypergraph, part: &mut [u8], imbalance: f64) {
    let max_passes = 10;
    for _ in 0..max_passes {
        if !fm_refine_pass(hg, part, imbalance) {
            break;
        }
    }
}
