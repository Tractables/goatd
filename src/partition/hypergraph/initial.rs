//! Partitions the coarsest hypergraph from nothing, the one phase of the sweep with
//! no partition to start from.
//!
//! Two generators, several restarts each: growing side 0 out from a random
//! seed, and random bisections cleaned up by FM. The best hyperedge cut among
//! them is what uncoarsening starts from, so this is also the only place the
//! module picks between whole partitions rather than single moves.

use super::model::Hypergraph;
use super::refine_fm::{FmScratch, refine_level};
use crate::partition::common::{BisectionStop, select_initial_partition};
use crate::rng::Xorshift64;

/// Grows side 0 outward from `seed` until it holds half the vertex weight;
/// everything else lands on side 1.
///
/// The gain of a candidate is recomputed from `pins_on_zero` on every step rather
/// than updated incrementally, which costs a scan of every unplaced vertex's
/// incidences per step but keeps the score exact. See "Where the two bisectors
/// differ" in the shared partition bookkeeping.
pub(super) fn greedy_growing(hg: &Hypergraph, seed: usize, stop: &mut BisectionStop) -> Vec<u8> {
    let n = hg.vertex_count;
    let total_weight: u32 = hg.vertex_weights.iter().sum();
    let target = total_weight / 2;

    let mut part = vec![1u8; n];
    let mut in_set = vec![false; n];
    let mut pins_on_zero: Vec<u32> = vec![0; hg.num_hyperedges()];

    part[seed] = 0;
    in_set[seed] = true;
    let mut set_weight = hg.vertex_weights[seed];

    for &hyperedge in hg.vertex_hyperedges(seed) {
        pins_on_zero[hyperedge as usize] += 1;
    }

    while set_weight < target {
        // A step scans every unplaced vertex's incidences, so the cost of one
        // is the cost of a pass over the hypergraph and the clock is read
        // here, as the graph sibling reads it.
        if stop.reached() {
            break;
        }
        let mut best_v = None;
        let mut best_gain = i64::MIN;

        for (v, &grown) in in_set.iter().enumerate() {
            if grown {
                continue;
            }
            let gain = vertex_gain_on_zero(hg, v, &pins_on_zero);
            if best_v.is_none() || gain > best_gain {
                best_gain = gain;
                best_v = Some(v);
            }
        }

        let Some(v) = best_v else {
            break;
        };
        part[v] = 0;
        in_set[v] = true;
        set_weight += hg.vertex_weights[v];

        for &hyperedge in hg.vertex_hyperedges(v) {
            pins_on_zero[hyperedge as usize] += 1;
        }
    }

    part
}

fn vertex_gain_on_zero(hg: &Hypergraph, v: usize, pins_on_zero: &[u32]) -> i64 {
    // Only two transitions matter for a hyperedge when `v` joins side
    // 0: it had no pin there and now straddles the split (-w), or it
    // was one pin short of complete and is now wholly inside (+w).
    // Everything in between leaves the cut where it was.
    let mut gain = 0i64;
    for &hyperedge in hg.vertex_hyperedges(v) {
        let hyperedge = hyperedge as usize;
        let weight = i64::from(hg.hyperedge_weights[hyperedge]);
        let total_pins = hg.hyperedge_size(hyperedge);
        let count0 = pins_on_zero[hyperedge];
        if count0 == 0 {
            gain -= weight;
        }
        if count0 + 1 == total_pins {
            gain += weight;
        }
    }
    gain
}

/// Summed weight of the hyperedges with pins on both sides.
///
/// A hyperedge is charged once however its pins are spread, so this is the
/// hyperedge-count metric rather than anything that grows with how many pins
/// lie on each side.
pub(super) fn hyperedge_cut(hg: &Hypergraph, part: &[u8]) -> u32 {
    let mut cut_weight = 0u32;
    for hyperedge in 0..hg.num_hyperedges() {
        let pins = hg.charged_hyperedge_pins(hyperedge);
        let first = part[pins[0] as usize];
        if pins.iter().any(|&v| part[v as usize] != first) {
            cut_weight += hg.hyperedge_weights[hyperedge];
        }
    }
    cut_weight
}

pub(super) fn initial_partition(
    hg: &Hypergraph,
    rng: &mut Xorshift64,
    max_imbalance: f64,
    scratch: &mut FmScratch,
    stop: &mut BisectionStop,
) -> Vec<u8> {
    // Restart counts: see "Where the two bisectors differ" in the shared
    // partition bookkeeping.
    let restarts = if hg.vertex_count >= 30 { 6 } else { 4 };
    select_initial_partition(
        &hg.vertex_weights,
        restarts,
        rng,
        stop,
        |seed, stop| greedy_growing(hg, seed, stop),
        |part, stop| refine_level(hg, part, max_imbalance, scratch, stop),
        |part| u64::from(hyperedge_cut(hg, part)),
    )
}
