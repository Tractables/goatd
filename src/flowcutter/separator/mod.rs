//! Pure-Rust port of FlowCutter's anytime balanced-separator search: the
//! algorithm in `vendor/treedecomp/upstream/flow-cutter-pace17/src/` plus the
//! `IFlowCutter::computeSeparator` driver in `vendor/treedecomp/upstream/IFlowCutter.cpp`.
//!
//! It does not match the C++ output bit-for-bit: the RNG and BFS tie-breaking
//! differ, and `cutter_count` is pinned to 1 rather than incremented every 16
//! iterations.
//!
//! This is the separator subroutine, not a tree decomposer. [`find`] returns
//! one balanced vertex separator and the two sides it separates. Full
//! tree-decomposition construction uses the C++ backend in
//! [`crate::flowcutter`].
//!

use std::time::{Duration, Instant};

use super::duration_ms;
use crate::rng::{SEED_OFFSET, Xorshift64};
use crate::{Error, Graph};

mod cutter;
mod expanded;
mod graph;
mod result;
mod search;
use cutter::*;
use expanded::*;
use graph::*;
pub(crate) use search::Search;

pub use result::Separator;
pub(crate) use result::with_sides;

/// `expanded` assigns two nodes per vertex and two arcs per original directed
/// arc, all in one `u32` index space.
const MAX_EXPANDED_BASE: u64 = u32::MAX as u64 / 2;

pub(crate) fn validate_graph_size(num_vertices: u32, num_edges: usize) -> Result<(), Error> {
    let num_edges = u64::try_from(num_edges).unwrap_or(u64::MAX);
    let expanded_base = u64::from(num_vertices).saturating_add(num_edges.saturating_mul(2));
    if expanded_base > MAX_EXPANDED_BASE {
        return Err(Error::TooLarge(format!(
            "graph is too large for the FlowCutter separator index space ({num_vertices} vertices and {num_edges} edges)"
        )));
    }
    Ok(())
}

/// Work limits for one separator search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct Budget {
    steps: u64,
    iterations: u32,
    timeout: Option<Duration>,
}

impl Budget {
    /// A deterministic search bounded by computation steps and outer
    /// iterations.
    pub const fn new(steps: u64, iterations: u32) -> Self {
        Self {
            steps,
            iterations,
            timeout: None,
        }
    }

    /// Add an elapsed-time limit. Nonzero sub-millisecond durations are
    /// rounded up to one millisecond.
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

/// Compute one FlowCutter separator and the two sides it separates.
///
/// Returns `Ok(None)` for a degenerate or disconnected graph, or when the
/// search finds no non-trivial separator within `budget`.
///
/// # Errors
///
/// Returns an error when a work limit is zero or unrepresentable, or when the
/// expanded graph does not fit the implementation's index space.
pub fn find(graph: &Graph, budget: Budget) -> Result<Option<Separator>, Error> {
    if budget.steps == 0 || budget.iterations == 0 {
        return Err(Error::InvalidInput(
            "a FlowCutter separator search needs positive steps and iterations".into(),
        ));
    }
    if budget.timeout.is_some_and(|timeout| timeout.is_zero()) {
        return Err(Error::InvalidInput(
            "a FlowCutter separator timeout must be positive".into(),
        ));
    }
    if budget.steps > i64::MAX as u64 {
        return Err(Error::InvalidInput(
            "FlowCutter separator step budget does not fit in i64".into(),
        ));
    }
    if budget.iterations > i32::MAX as u32 {
        return Err(Error::InvalidInput(
            "FlowCutter separator iteration count does not fit in i32".into(),
        ));
    }
    if budget
        .timeout
        .is_some_and(|timeout| timeout.as_millis() > i64::MAX as u128)
    {
        return Err(Error::InvalidInput(
            "FlowCutter separator timeout does not fit in milliseconds".into(),
        ));
    }
    validate_graph_size(graph.num_vertices, graph.edges.len())?;
    let steps = budget.steps as i64;
    let iterations = budget.iterations as i32;
    let timeout_ms = budget.timeout.map(duration_ms).unwrap_or(0);
    let separator = compute_vertices(
        graph.num_vertices as usize,
        &graph.edges,
        steps,
        iterations,
        timeout_ms,
    );
    Ok(separator.and_then(|separator| result::with_sides(graph, separator)))
}

/// Milliseconds of construction work since `start`, on whichever clock the
/// meter is serving.
///
/// Outside a metered construction that is the real wall, and every caller
/// behaves exactly as it did before the meter existed. Inside one it is charged
/// work, so a loop cannot outrun a budget that was measured against the same
/// clock.
#[inline]
fn elapsed_since(start: Instant) -> u128 {
    crate::meter::now()
        .saturating_duration_since(start)
        .as_millis()
}

/// Compute one balanced vertex separator using FlowCutter's anytime
/// max-flow / Pareto-balance search.
///
/// `edges` is a list of undirected edges as (u, v) pairs (0-indexed,
/// u < n, v < n, u != v).  Self-loops and duplicates are tolerated but not
/// recommended.  The returned separator is a sorted list of vertex IDs
/// in the input space.
///
/// `steps` is the soft step budget (subtracted by `sqrt(n)*sqrt(2m)/50` per
/// iteration).  `iters` caps the number of outer iterations.
/// A zero internal timeout disables the wall-clock deadline. The public
/// [`Budget`] represents that as `timeout: None`.
///
/// Returns `None` if the input is degenerate (n < 3, disconnected, or no
/// non-trivial separator found within budget).
fn compute_vertices(
    n: usize,
    edges: &[(u32, u32)],
    steps: i64,
    iters: i32,
    timeout_ms: i64,
) -> Option<Vec<u32>> {
    let mut search = Search::new(n, edges, steps, iters)?;
    let start = crate::meter::now();
    while !search.exhausted()
        && !crate::deadline::expired(None)
        && (timeout_ms <= 0 || elapsed_since(start) < timeout_ms as u128)
    {
        let _ = search.step(false);
    }
    search.into_vertices()
}

/// Recover the original-space vertex separator (not expanded-space) from the
/// current cut.
fn extract_original_separator(g: &OrigGraph, a_orig: u32, multi: &MultiCutter) -> Vec<u32> {
    let mut sep: Vec<u32> = Vec::new();
    let cur_cut = multi.current_cut();

    for &xy in cur_cut {
        if is_intra(xy, a_orig) {
            sep.push(intra_to_orig_node(xy, a_orig));
        }
    }

    let n_orig = g.n as usize;
    // Expanded-space smaller-side count double-counts each vertex (in + out);
    // `sep` here is single-counted, hence -sep.len() then /2 for vertex count.
    let cur_small = multi.current_smaller_size() as i64;
    let mut left_size = (cur_small - sep.len() as i64) / 2;
    let mut right_size = n_orig as i64 - sep.len() as i64 - left_size;

    let is_orig_left = |x: u32| -> bool {
        // Tests via the OUT node: the expanded graph's "left" (smaller) side
        // holds u_out for u in the original left set, not u_in.
        multi.is_on_smaller_side(orig_node_to_exp(x, true))
    };

    for &xy in cur_cut {
        if !is_intra(xy, a_orig) {
            let lr = inter_to_orig_arc(xy);
            let mut l = g.tail[lr as usize];
            let mut r = g.head[lr as usize];
            if is_orig_left(r) {
                std::mem::swap(&mut l, &mut r);
            }
            if left_size > right_size {
                sep.push(l);
                left_size -= 1;
            } else {
                sep.push(r);
                right_size -= 1;
            }
        }
    }

    sep.sort_unstable();
    sep.dedup();
    sep
}

/// The source and sink pairs the cutter runs between, drawn from the crate's
/// own generator like every other seeded search here.
///
/// `rand`'s documentation says its algorithms may change in any release,
/// which would move this pass's answer between two versions of the crate that
/// are otherwise the same. A separator is documented as a function of the
/// graph and the seed, so the stream has to be one the crate owns.
fn select_random_st_pairs(n: u32, count: u32, seed: u64) -> Vec<(u32, u32)> {
    let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
    let mut out = Vec::with_capacity(count as usize);
    if n < 2 {
        return out;
    }
    for _ in 0..count {
        let mut s;
        let mut t;
        loop {
            s = rng.below(n as usize) as u32;
            t = rng.below(n as usize) as u32;
            if s != t {
                break;
            }
        }
        out.push((s, t));
    }
    out
}

/// LCG params (a=48271, m=2^31-1) match C++ std::minstd_rand.
struct MinstdRand {
    state: u64,
}

impl MinstdRand {
    fn new(seed: u32) -> Self {
        // libstdc++'s linear_congruential_engine treats seed 0 as seed 1;
        // match that so the sequence lines up with minstd_rand.
        let s = if seed == 0 { 1 } else { seed };
        MinstdRand { state: s as u64 }
    }
    fn next(&mut self) -> u32 {
        self.state = (self.state * 48271) % ((1u64 << 31) - 1);
        self.state as u32
    }
}

#[cfg(test)]
mod tests;
