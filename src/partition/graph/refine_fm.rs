//! Fiduccia-Mattheyses refinement of an existing graph bisection: bucket-queue global
//! passes at every level, plus localized passes seeded on the boundary at the
//! finest one.
//!
//! A pass moves vertices one at a time and never moves the same vertex twice,
//! then keeps the best *prefix* of that move sequence and rolls the rest back.
//! Negative-gain moves are allowed, but survive rollback only if later moves
//! make their prefix profitable. This lets FM leave a single-move local
//! minimum.

use std::collections::VecDeque;

use super::csr::CsrGraph;
use crate::partition::common::{
    BisectionStop, FmBalance, GainBuckets, Stall, commit_best_prefix, fm_balance,
};

pub(super) struct FmScratch {
    gain: Vec<i64>,
    cut_edges: Vec<i64>,
    locked: Vec<bool>,
    moves: Vec<usize>,
    cumulative_gain: Vec<i64>,
    bq: [GainBuckets; 2],
    region: RegionScratch,
}

impl FmScratch {
    pub(super) fn new() -> Self {
        FmScratch {
            gain: Vec::new(),
            cut_edges: Vec::new(),
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
        self.cut_edges.clear();
        self.cut_edges.resize(n, 0);
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
/// per-vertex arrays are cleared over the region the pass built rather than
/// over the whole graph. That leaves every array as the pass found it, which is
/// what lets `prepare` be a resize that usually does nothing.
pub(super) struct RegionScratch {
    gain: Vec<i64>,
    in_region: Vec<bool>,
    locked: Vec<bool>,
    /// Whether a vertex has a neighbour on the other side, for the span of one
    /// pass: 0 not yet computed, 1 interior, 2 on the boundary.
    boundary: Vec<u8>,
    /// The vertices whose `boundary` entry this pass wrote.
    boundary_touched: Vec<usize>,
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
            boundary: Vec::new(),
            boundary_touched: Vec::new(),
            region_list: Vec::new(),
            queue: VecDeque::new(),
            moves: Vec::new(),
            cumulative_gain: Vec::new(),
        }
    }

    /// Size the per-vertex arrays for a graph of `n` vertices and empty the
    /// lists. The arrays keep the values they hold, which the previous pass
    /// left cleared; a length that grows is filled with the same cleared value.
    fn prepare(&mut self, n: usize) {
        self.gain.resize(n, 0);
        self.in_region.resize(n, false);
        self.locked.resize(n, false);
        self.boundary.resize(n, 0);
        // The pass that ends leaves these as it found them. `gain` is exempt:
        // every region vertex is written before it is read, and no other entry
        // is read at all.
        debug_assert!(self.in_region.iter().all(|&member| !member));
        debug_assert!(self.locked.iter().all(|&locked| !locked));
        debug_assert!(self.boundary.iter().all(|&state| state == 0));
        self.boundary_touched.clear();
        self.region_list.clear();
        self.queue.clear();
        self.moves.clear();
        self.cumulative_gain.clear();
    }
}

/// Returns true if the partition was improved.
///
/// On `false` the partition is restored to exactly what was passed in, so a
/// caller can loop on this until it stops paying off without keeping a copy.
pub(super) fn fm_refine_pass(
    graph: &CsrGraph,
    part: &mut [u8],
    max_imbalance: f64,
    scratch: &mut FmScratch,
    stop: &mut BisectionStop,
) -> bool {
    // One pass: the gain build below walks the whole graph, and the move loop
    // walks the neighbourhood of everything it moves.
    crate::meter::charge(graph.pass_units());
    let n = graph.num_vertices();
    let Some(FmBalance {
        mut weight,
        min_part_weight,
        max_part_weight,
    }) = fm_balance(n, &graph.vertex_weights, part, max_imbalance)
    else {
        return false;
    };

    scratch.prepare(n);
    let gain = scratch.gain.as_mut_slice();
    let cut_edges = scratch.cut_edges.as_mut_slice();
    for v in 0..n {
        let my_part = part[v];
        let start = graph.offsets[v] as usize;
        let end = graph.offsets[v + 1] as usize;
        let nbrs = &graph.neighbors[start..end];
        let weights = &graph.edge_weights[start..end];
        let mut g = 0i64;
        let mut cut = 0i64;
        for (&nb, &w) in nbrs.iter().zip(weights) {
            let w = i64::from(w);
            if part[nb as usize] != my_part {
                g += w;
                cut += w;
            } else {
                g -= w;
            }
        }
        gain[v] = g;
        cut_edges[v] = cut;
    }

    // Only boundary vertices are queued: an interior vertex has every edge on
    // its own side, so moving it can only add to the cut.
    let bq = &mut scratch.bq;
    for v in 0..n {
        if cut_edges[v] > 0 {
            bq[part[v] as usize].insert(v, gain[v]);
        }
    }

    let locked = scratch.locked.as_mut_slice();
    let moves = &mut scratch.moves;
    let cumulative_gain = &mut scratch.cumulative_gain;
    let mut running_gain: i64 = 0;
    let mut stall = Stall::new((n / 2).max(20));

    for _ in 0..n {
        if stop.reached() {
            break;
        }
        let mut best_v: Option<usize> = None;
        let mut best_gain = i64::MIN;
        let mut best_from: usize = 0;

        for side in 0..2 {
            let to = 1 - side;
            // Every queued vertex weighs at least one, so a side at the floor
            // can give none up and a side at the ceiling can take none. Without
            // this the search walks that side's whole queue to return nothing,
            // once per move, which is where a pass sits once it drifts to the
            // balance boundary.
            if weight[side] <= min_part_weight || weight[to] >= max_part_weight {
                continue;
            }
            let candidate = bq[side].best_satisfying(|vertex| {
                !locked[vertex]
                    && weight[side] - graph.vertex_weights[vertex] >= min_part_weight
                    && weight[to] + graph.vertex_weights[vertex] <= max_part_weight
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
        weight[from] -= graph.vertex_weights[v];
        weight[to] += graph.vertex_weights[v];
        part[v] = to as u8;
        locked[v] = true;

        running_gain += best_gain;
        moves.push(v);
        cumulative_gain.push(running_gain);

        if stall.record(running_gain) {
            break;
        }

        let v_start = graph.offsets[v] as usize;
        let v_end = graph.offsets[v + 1] as usize;
        let v_nbrs = &graph.neighbors[v_start..v_end];
        let v_weights = &graph.edge_weights[v_start..v_end];
        for (&nb_raw, &w_raw) in v_nbrs.iter().zip(v_weights) {
            let nb = nb_raw as usize;
            if locked[nb] {
                continue;
            }
            let w = i64::from(w_raw);
            let nb_part = part[nb] as usize;
            let was_in_queue = bq[nb_part].contains(nb);
            // `2 * w`, not `w`: `gain` is external weight minus internal
            // weight, and this edge crossed from one of those sums to the
            // other, so their difference moves by twice the edge's weight.
            // `cut_edges` is the external sum on its own, so it moves by `w`.
            if nb_part == to {
                gain[nb] -= 2 * w;
                cut_edges[nb] -= w;
            } else {
                gain[nb] += 2 * w;
                cut_edges[nb] += w;
            }
            let on_boundary = cut_edges[nb] > 0;
            if on_boundary {
                if was_in_queue {
                    bq[nb_part].update(nb, gain[nb]);
                } else {
                    bq[nb_part].insert(nb, gain[nb]);
                }
            } else if was_in_queue {
                bq[nb_part].remove(nb);
            }
        }
    }

    commit_best_prefix(moves, cumulative_gain, part)
}

/// FM confined to a region grown around `seed`, run at the finest level after
/// the global passes have stopped improving.
///
/// The region is capped, so selection is a linear scan over its vertices rather
/// than a bucket queue. Gains exist only for region vertices and a move updates
/// only region neighbours; a vertex outside the region is never a candidate, so
/// its gain entry stays at zero.
pub(super) fn localized_fm_pass(
    graph: &CsrGraph,
    part: &mut [u8],
    seed: usize,
    max_imbalance: f64,
    scratch: &mut RegionScratch,
    stop: &mut BisectionStop,
) -> bool {
    let n = graph.num_vertices();
    let Some(FmBalance {
        mut weight,
        min_part_weight,
        max_part_weight,
    }) = fm_balance(n, &graph.vertex_weights, part, max_imbalance)
    else {
        return false;
    };

    scratch.prepare(n);
    let in_region = scratch.in_region.as_mut_slice();
    let boundary = scratch.boundary.as_mut_slice();
    let boundary_touched = &mut scratch.boundary_touched;
    let region_list = &mut scratch.region_list;
    let queue = &mut scratch.queue;

    // region_list is collected alongside in_region to avoid a separate O(n)
    // scan in the selection loop below.
    let max_region = (n / 4).max(20).min(n);
    in_region[seed] = true;
    queue.push_back(seed);
    region_list.push(seed);

    // Cross-partition neighbours first, so the region hugs the cut even when
    // the cap cuts the growth short.
    while let Some(v) = queue.pop_front() {
        if region_list.len() >= max_region {
            break;
        }
        for &nb in graph.neighbors(v) {
            let nb = nb as usize;
            if !in_region[nb] && part[nb] != part[v] && region_list.len() < max_region {
                in_region[nb] = true;
                queue.push_back(nb);
                region_list.push(nb);
            }
        }
        // Region also absorbs same-side neighbors of boundary vertices, not
        // just cross-partition ones — gives FM room to move on both sides.
        for &nb in graph.neighbors(v) {
            let nb = nb as usize;
            if !in_region[nb] && region_list.len() < max_region {
                // No move happens until the region is grown, so whether `nb`
                // has a neighbour on the other side does not change while this
                // loop runs. A vertex the region keeps reaching but never
                // admits is reached once per region neighbour it has, and
                // without the record below its adjacency is walked every time.
                let is_boundary = match boundary[nb] {
                    0 => {
                        let nb_part = part[nb];
                        let on_boundary = graph
                            .neighbors(nb)
                            .iter()
                            .any(|&nnb| part[nnb as usize] != nb_part);
                        boundary[nb] = 1 + u8::from(on_boundary);
                        boundary_touched.push(nb);
                        on_boundary
                    }
                    known => known == 2,
                };
                if is_boundary {
                    in_region[nb] = true;
                    queue.push_back(nb);
                    region_list.push(nb);
                }
            }
        }
    }

    // Gains only for region vertices: O(region × deg), not O(n × deg). A
    // non-region entry is never read, so `gain` is written here and left alone
    // rather than cleared.
    let gain = scratch.gain.as_mut_slice();
    for &v in region_list.iter() {
        let my_part = part[v];
        let start = graph.offsets[v] as usize;
        let end = graph.offsets[v + 1] as usize;
        let nbrs = &graph.neighbors[start..end];
        let weights = &graph.edge_weights[start..end];
        let mut g = 0i64;
        for (&nb, &w) in nbrs.iter().zip(weights) {
            let w = i64::from(w);
            if part[nb as usize] != my_part {
                g += w;
            } else {
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

    // O(region²), not O(n × region): only region_list is scanned per move.
    // Ties go to whichever vertex BFS reached first because `gain[v] > best_g`
    // is strict. The hypergraph pass scans its region in ascending index order.
    for _ in 0..region_list.len() {
        if stop.reached() {
            break;
        }
        let mut best_v = None;
        let mut best_g = i64::MIN;
        for &v in region_list.iter() {
            if locked[v] {
                continue;
            }
            let from = part[v] as usize;
            let to = 1 - from;
            let nfw = weight[from] - graph.vertex_weights[v];
            let ntw = weight[to] + graph.vertex_weights[v];
            if nfw < min_part_weight || ntw > max_part_weight {
                continue;
            }
            if best_v.is_none() || gain[v] > best_g {
                best_g = gain[v];
                best_v = Some(v);
            }
        }
        let Some(v) = best_v else {
            break;
        };
        let from = part[v] as usize;
        let to = 1 - from;
        weight[from] -= graph.vertex_weights[v];
        weight[to] += graph.vertex_weights[v];
        part[v] = to as u8;
        locked[v] = true;

        running_gain += best_g;
        moves.push(v);
        cumulative_gain.push(running_gain);

        if stall.record(running_gain) {
            break;
        }

        let v_start = graph.offsets[v] as usize;
        let v_end = graph.offsets[v + 1] as usize;
        let v_nbrs = &graph.neighbors[v_start..v_end];
        let v_weights = &graph.edge_weights[v_start..v_end];
        for (&nb_raw, &w_raw) in v_nbrs.iter().zip(v_weights) {
            let nb = nb_raw as usize;
            if locked[nb] || !in_region[nb] {
                continue;
            }
            let w = i64::from(w_raw);
            if part[nb] == to as u8 {
                gain[nb] -= 2 * w;
            } else {
                gain[nb] += 2 * w;
            }
        }
    }

    let improved = commit_best_prefix(moves, cumulative_gain, part);

    // Hand the arrays back the way they were found. Only region vertices are
    // marked in `in_region` and `locked`, and `boundary_touched` names every
    // vertex whose `boundary` entry was written.
    for &v in region_list.iter() {
        in_region[v] = false;
        locked[v] = false;
    }
    for &v in boundary_touched.iter() {
        boundary[v] = 0;
    }
    improved
}

/// Standard FM refinement (global passes only).
///
/// Passes repeat until one fails to improve. The cap bounds the case where each
/// pass finds a single-move improvement and would otherwise keep going.
pub(super) fn refine_level(
    graph: &CsrGraph,
    part: &mut [u8],
    max_imbalance: f64,
    scratch: &mut FmScratch,
    stop: &mut BisectionStop,
) {
    let max_passes = 10;
    for _ in 0..max_passes {
        if !fm_refine_pass(graph, part, max_imbalance, scratch, stop) || stop.stopped() {
            break;
        }
    }
}

/// FM refinement with multi-try localized passes after global FM.
pub(super) fn refine_finest_level(
    graph: &CsrGraph,
    part: &mut [u8],
    max_imbalance: f64,
    scratch: &mut FmScratch,
    stop: &mut BisectionStop,
) {
    refine_level(graph, part, max_imbalance, scratch, stop);
    if stop.stopped() {
        return;
    }

    let n = graph.num_vertices();
    if n < 20 {
        return;
    }
    let num_tries = 4.min(n);
    let mut boundary: Vec<usize> = Vec::new();
    for v in 0..n {
        let my_part = part[v];
        if graph
            .neighbors(v)
            .iter()
            .any(|&nb| part[nb as usize] != my_part)
        {
            boundary.push(v);
        }
    }
    if boundary.is_empty() {
        return;
    }

    // 7919 is prime, so successive tries land in unrelated stretches of the
    // boundary list rather than in one region's worth of adjacent vertices.
    for i in 0..num_tries {
        if stop.stopped() {
            break;
        }
        let seed = boundary[(i * 7919) % boundary.len()];
        localized_fm_pass(graph, part, seed, max_imbalance, &mut scratch.region, stop);
    }
}
