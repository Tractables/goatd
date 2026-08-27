//! The portfolio executor: run a portfolio of elimination configs under a
//! wall-clock budget and keep per-slot outcomes.
//!
//! The slots themselves come from [`super::width_opt`]; the single-order
//! construction in [`super::elimination_td`] does not go through here.

use std::time::{Duration, Instant};

use super::minfill_core::{ElimExit, ElimStop};
use super::{refine, width_opt};
use crate::deadline::expired;
use crate::flowcutter::{FcBudget, flowcutter_td};
use crate::{Graph, TreeDecomposition};

/// Vertices past which the trailing FlowCutter slot is not run.
const FC_SLOT_MAX_VERTICES: u32 = 100_000;
/// Cap on the FC slot's wall-clock budget. Picked to keep overhead bounded
/// within the `[soft_deadline, hard_deadline]` margin (hard_deadline = 2×soft)
/// without starving any downstream work.
const FC_SLOT_CAP_MS: i64 = 2_000;
/// Exit early if FlowCutter hasn't improved treewidth for this long. Caps
/// per-graph overhead where FC converges fast.
const FC_SLOT_PATIENCE_MS: i64 = 500;

/// Per-slot soft cap on `Config::MinFill` (eager fill-count recompute is
/// 10–20× more expensive per step than the lazy variants). Applied
/// unconditionally so a run with no portfolio deadline still bounds MinFill at
/// 1 s.
const MINFILL_SLOT_MAX_MS: u64 = 1_000;

/// Cap on refinement-phase seeds when there is no deadline. 100 is the knee
/// measured across benchmark graphs: fewer leaves quality on the table, more
/// costs construction time without improving the decomposition.
const MAX_REFINE_SLOTS: u64 = 100;

/// Soft deadline (ms) of the single-slot portfolio. The hard deadline inside
/// the elimination core is `2×` this. Measured knee across benchmark graphs:
/// sub-second budgets give a worse decomposition on hard instances, and
/// budgets above ~2 s do not improve it further.
const SINGLE_SLOT_TIMEOUT_MS: u64 = 1000;

/// What a portfolio runs under, beyond the slot list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortfolioConfig {
    /// Soft portfolio deadline. `None` runs to completion with no between-slot
    /// deadline. `Some(N)` puts the soft deadline at `start+N` ms and the hard
    /// one at `start+2N`; the elimination core emergency-bails to a path
    /// decomposition once the hard deadline passes.
    pub timeout_ms: Option<u64>,
    /// Cap on refinement-phase sampling seeds.
    pub refine_cap: u64,
    /// Wall-clock cap of a trailing FlowCutter slot. `None` runs no such
    /// slot. `Some(cap_ms)` runs FlowCutter once, after the sampling
    /// refinement, for up to `min(remaining hard budget, cap_ms)` ms, and the
    /// decomposition it finds is one more candidate.
    pub fc_slot_cap_ms: Option<i64>,
}

impl PortfolioConfig {
    /// The single-slot portfolio's configuration: 1 s soft / 2 s hard deadline,
    /// 100 refinement samples, and a 2 s trailing FlowCutter slot when
    /// `fc_slot` is set.
    pub fn single_slot(fc_slot: bool) -> Self {
        Self {
            timeout_ms: Some(SINGLE_SLOT_TIMEOUT_MS),
            refine_cap: MAX_REFINE_SLOTS,
            fc_slot_cap_ms: fc_slot.then_some(FC_SLOT_CAP_MS),
        }
    }

    /// The five-slot portfolio's configuration: `timeout_ms` as the soft
    /// deadline (`None` runs to completion), 100 refinement samples, and no
    /// trailing FlowCutter slot — the refined entry point ends in a FlowCutter
    /// refinement pass instead.
    pub fn five_slot(timeout_ms: Option<u64>) -> Self {
        Self {
            timeout_ms,
            refine_cap: MAX_REFINE_SLOTS,
            fc_slot_cap_ms: None,
        }
    }
}

fn is_mindegree_variant(c: width_opt::Config<'_>) -> bool {
    matches!(
        c,
        width_opt::Config::MinDegree | width_opt::Config::MinDegreeSampled { .. }
    )
}

/// One slot's outcome inside one run of `run_portfolio`.
enum SlotResult {
    /// A decomposition and its width.
    Produced { td: TreeDecomposition, width: u32 },
    /// A slot that produced nothing: skipped, out of time, or over the width
    /// bound.
    Empty,
}

impl SlotResult {
    fn from_td(td: TreeDecomposition) -> Self {
        SlotResult::Produced {
            width: td.treewidth(),
            td,
        }
    }

    /// The width this slot reached, or the maximum for a slot that produced
    /// nothing — the value that loses against every real width.
    fn width_or_max(&self) -> u32 {
        match self {
            SlotResult::Produced { width, .. } => *width,
            SlotResult::Empty => u32::MAX,
        }
    }
}

/// Fill remaining portfolio slots with empty stubs starting at index
/// `from_idx`. Called after `hard_deadline` trips so all downstream slots get
/// a recorded stub without re-entering elimination.
fn emit_skipped_stubs(
    results: &mut Vec<SlotResult>,
    portfolio: &[(width_opt::Config<'_>, u64)],
    from_idx: usize,
) {
    let skipped = portfolio.len().saturating_sub(from_idx);
    results.extend(std::iter::repeat_with(|| SlotResult::Empty).take(skipped));
}

/// Intentionally does not touch `best_width` — the width bound exists to cut
/// elimination runs short, and FC is not one.
fn run_fc_slot(
    graph: &Graph,
    cap_ms: i64,
    hard_deadline: Option<Instant>,
    results: &mut Vec<SlotResult>,
) {
    let remaining_ms: i64 = hard_deadline
        .map(|hd| crate::deadline::remaining(hd).as_millis() as i64)
        .unwrap_or(cap_ms);
    let fc_timeout = remaining_ms.max(1).min(cap_ms);
    // Skip windows too small to seed useful FC iterations — FFI overhead
    // alone eats tens of ms on small graphs.
    if fc_timeout < 50 {
        return;
    }
    match flowcutter_td(graph, FcBudget::timed(fc_timeout, FC_SLOT_PATIENCE_MS, 50)) {
        Ok(fc_td) => results.push(SlotResult::from_td(fc_td)),
        Err(_) => results.push(SlotResult::Empty),
    }
}

/// What one portfolio slot's elimination left behind.
enum SlotOutcome {
    /// A decomposition, recorded and folded into the best width so far.
    Produced,
    /// A bag passed the width bound, so nothing usable came back. That bound
    /// comes from a slot that already produced one, so a winner exists.
    WidthAborted,
    /// The hard deadline stopped the elimination and no emergency fill was
    /// asked for, so the partial bags are not a decomposition.
    Bailed,
}

/// Record what one slot came back with — a decomposition, or the stub that
/// stands for a slot with nothing to offer — and fold a produced width into
/// `best_width`.
///
/// `force_emit` is what the slot asked of the elimination: with it, a
/// hard-deadline bail still leaves a complete (wide) decomposition behind, and
/// so counts as produced.
fn record_slot(
    run: width_opt::ConfigRun,
    force_emit: bool,
    results: &mut Vec<SlotResult>,
    best_width: &mut Option<u32>,
) -> SlotOutcome {
    match run.exit {
        ElimExit::WidthAborted => {
            results.push(SlotResult::Empty);
            SlotOutcome::WidthAborted
        }
        ElimExit::DeadlineBailed if !force_emit => {
            results.push(SlotResult::Empty);
            SlotOutcome::Bailed
        }
        ElimExit::Natural | ElimExit::DeadlineBailed => {
            let slot = SlotResult::from_td(run.td);
            let w = slot.width_or_max();
            *best_width = Some(best_width.map_or(w, |b| b.min(w)));
            results.push(slot);
            SlotOutcome::Produced
        }
    }
}

/// Builds a run's slot list for a seed, given the weight vector.
type PortfolioBuilder = for<'w> fn(u64, &'w [u32]) -> Vec<(width_opt::Config<'w>, u64)>;

/// Run a portfolio: large-residual skip on non-MinDegree configs, 1 s soft cap
/// on `Config::MinFill`, then refinement samples with the remaining budget,
/// then the trailing FlowCutter slot where configured. Returns one
/// `SlotResult` per slot, plus one per refinement sample and one for the
/// FlowCutter slot where those run.
///
/// At least one of them always carries a TD: slot 0 is exempt from both
/// between-slot skips, runs with no `width_bound` (so it cannot abort on
/// width) and with `force_emit` set (so a deadline bail is completed by an
/// emergency path decomposition rather than discarded).
///
/// `weight` has one entry per vertex and is what the sampling orders draw tie
/// sets with; every slot and every refinement sample shares it.
fn run_portfolio(
    graph: &Graph,
    weight: &[u32],
    seed: u64,
    portfolio: PortfolioBuilder,
    cfg: PortfolioConfig,
) -> Vec<SlotResult> {
    let start = crate::meter::now();
    let deadline: Option<Instant> = cfg.timeout_ms.map(|ms| start + Duration::from_millis(ms));
    // Twice the soft timeout: enough headroom past it for the emergency bail to
    // assemble a decomposition and return it.
    let hard_deadline: Option<Instant> = cfg
        .timeout_ms
        .map(|ms| start + Duration::from_millis(ms.saturating_mul(2)));
    let prebuilt = width_opt::prebuild(graph.num_vertices, &graph.edges);
    let portfolio = portfolio(seed, weight);
    let large_residual = prebuilt.num_active() > width_opt::NESTED_DISS_MAX_ACTIVE;
    let mut results: Vec<SlotResult> = Vec::with_capacity(5);

    // Width of the best TD seen so far, and the bound every later slot is held
    // to. `None` means nothing has been produced yet, which is what makes slot
    // 0 run unbounded and what asks a slot for an emergency fill.
    let mut best_width: Option<u32> = None;

    // Anytime early-exit flag: set when any slot's elimination
    // emergency-bailed on `hard_deadline`, when we observe `hard_deadline`
    // expired between slots, or when slot 0 itself emergency-bailed. Breaks
    // out of the main + refinement loops so remaining slots don't re-enter
    // elimination only to immediately emergency-bail again.
    let mut hard_deadline_tripped = false;

    for (i, (config, s)) in portfolio.iter().copied().enumerate() {
        // Honour the deadline between configs (when set), but always run
        // config 0 so we return something even on huge graphs that would
        // otherwise time out inside the first config.
        if i > 0 && expired(deadline) {
            results.push(SlotResult::Empty);
            continue;
        }
        // On large residuals, only min-degree variants reliably complete —
        // NestedDiss and MinFill variants can overshoot by seconds.
        if i > 0 && large_residual && !is_mindegree_variant(config) {
            results.push(SlotResult::Empty);
            continue;
        }
        let slot_start = crate::meter::now();
        // MinFill cap: with no portfolio deadline this is the only MinFill
        // bound; with one it tightens MinFill to min(slot_start + 1 s,
        // portfolio deadline).
        let soft_deadline = if config == width_opt::Config::MinFill {
            let cap = slot_start + Duration::from_millis(MINFILL_SLOT_MAX_MS);
            Some(deadline.map_or(cap, |d| d.min(cap)))
        } else {
            deadline
        };
        // Force an `emergency_path_decomp` fill only while no slot has
        // produced a usable TD yet. Once one has, a later slot's emergency
        // fill would be wasted work: its (wide) TD would lose lex-min
        // (width, tbs) to the existing winner anyway.
        let force_emit = best_width.is_none();
        let run = width_opt::run_config_prebuilt(
            &prebuilt,
            width_opt::RunSpec {
                config,
                seed: s,
                stop: ElimStop {
                    deadline: soft_deadline,
                    hard_deadline,
                    width_bound: best_width,
                },
                force_emit,
            },
        );
        hard_deadline_tripped = match record_slot(run, force_emit, &mut results, &mut best_width) {
            // Out of time, and the bags this slot holds are not a
            // decomposition — nothing later will fare better.
            SlotOutcome::Bailed => true,
            // Nothing usable from this slot, but the portfolio is still inside
            // its budget.
            SlotOutcome::WidthAborted => false,
            SlotOutcome::Produced => expired(hard_deadline),
        };
        if hard_deadline_tripped {
            emit_skipped_stubs(&mut results, &portfolio, i + 1);
            break;
        }
    }
    // Refinement phase: sample additional seeds of the htd-style sampling
    // config with any remaining budget. Measured ≥79% of min-fill pops have
    // ≥2 tied candidates, so different seeds explore genuinely different
    // elimination orders and can lower width on small/medium graphs where the
    // base portfolio returns in tens of ms. Falls back to MinDegreeSampled on
    // large residuals, matching the main loop's skip rule. Stops at `deadline`
    // when there is one, and at `refine_cap` samples regardless.
    let refine_config = if large_residual {
        width_opt::Config::MinDegreeSampled { weight }
    } else {
        width_opt::Config::MinFillSampled { weight }
    };
    let max_refine = cfg.refine_cap;
    let mut refine_k: u64 = 0;
    // Refinement hard-deadline guard: normally `deadline` fires before
    // `hard_deadline` (deadline + timeout_ms = hard_deadline), but we also
    // short-circuit when a prior slot emergency-bailed so refinement doesn't
    // waste budget re-running the same bail path.
    while refine_k < max_refine
        && !hard_deadline_tripped
        && !expired(deadline)
        && !expired(hard_deadline)
    {
        let refine_seed = seed.wrapping_add(100 + refine_k.wrapping_mul(7919));
        // Refinement only runs after plateau, which means at least one prior
        // slot won — no emergency fill needed here.
        let run = width_opt::run_config_prebuilt(
            &prebuilt,
            width_opt::RunSpec {
                config: refine_config,
                seed: refine_seed,
                stop: ElimStop {
                    deadline,
                    hard_deadline,
                    width_bound: best_width,
                },
                force_emit: false,
            },
        );
        match record_slot(run, false, &mut results, &mut best_width) {
            // No time left for more refine slots.
            SlotOutcome::Bailed => break,
            // A width-aborted seed keeps the refinement going: another seed
            // explores a different elimination order.
            SlotOutcome::Produced | SlotOutcome::WidthAborted => refine_k += 1,
        }
    }
    // Runs vanilla FlowCutter once as a final portfolio candidate. Placed after
    // the sampling-refinement loop so it runs in the remaining hard-deadline
    // margin without starving sampling — under a typical `timeout_ms`
    // contract, `hard_deadline` = 2×`soft_deadline`, leaving up to
    // `soft_deadline` of slack here. The FC TD is already FC-native, so no
    // further refinement is applied.
    if let Some(cap_ms) = cfg.fc_slot_cap_ms.filter(|_| {
        !hard_deadline_tripped
            && graph.num_vertices <= FC_SLOT_MAX_VERTICES
            && !expired(hard_deadline)
    }) {
        run_fc_slot(graph, cap_ms, hard_deadline, &mut results);
    }
    results
}

/// The decompositions a portfolio produced, in slot order, with the slots that
/// produced nothing left out.
fn produced(results: Vec<SlotResult>) -> Vec<TreeDecomposition> {
    results
        .into_iter()
        .filter_map(|r| match r {
            SlotResult::Produced { td, .. } => Some(td),
            SlotResult::Empty => None,
        })
        .collect()
}

/// Run the single-slot portfolio — one sampled min-fill slot, then up to
/// `cfg.refine_cap` further seeds of it, then the trailing FlowCutter slot
/// when `cfg` names one — and return every decomposition it produced, in
/// slot order. Never empty: slot 0 always produces one.
///
/// The caller picks among them; [`refined_select_key`] is the order the
/// refined path uses, and a caller with a cost of its own can rank by that.
pub fn single_slot_portfolio(
    graph: &Graph,
    weight: &[u32],
    seed: u64,
    cfg: PortfolioConfig,
) -> Vec<TreeDecomposition> {
    produced(run_portfolio(
        graph,
        weight,
        seed,
        width_opt::single_slot_portfolio,
        cfg,
    ))
}

/// Run the five-slot portfolio and return every decomposition it produced,
/// sorted ascending by [`refined_select_key`] with ties kept in slot order
/// (a stable sort), so the first is the portfolio's winner. Never empty.
pub fn five_slot_portfolio(
    graph: &Graph,
    weight: &[u32],
    seed: u64,
    cfg: PortfolioConfig,
) -> Vec<TreeDecomposition> {
    let mut tds = produced(run_portfolio(
        graph,
        weight,
        seed,
        width_opt::five_slot_portfolio,
        cfg,
    ));
    tds.sort_by_key(|td| refined_select_key(td.treewidth(), td.total_bag_size()));
    tds
}

/// The five-slot portfolio's winner, refined by FlowCutter cuts
/// ([`refine_td_with_flowcutter_cut`](super::refine_td_with_flowcutter_cut)).
///
/// `refine_deadline` bounds the refinement pass; it is absolute, so a
/// portfolio that spent its whole budget leaves the refinement no time rather
/// than doubling the cost. Both halves are anytime — the portfolio keeps the
/// best decomposition found so far, and a skipped refinement returns it
/// unchanged — so a bounded build still yields a decomposition.
pub fn refined_td(
    graph: &Graph,
    weight: &[u32],
    seed: u64,
    cfg: PortfolioConfig,
    refine_deadline: Option<Instant>,
) -> TreeDecomposition {
    let td = five_slot_portfolio(graph, weight, seed, cfg)
        .into_iter()
        .next()
        .expect("slot 0 always produces a decomposition");
    let all_vertices: Vec<u32> = (0..graph.num_vertices).collect();
    refine::refine_td_with_flowcutter_cut(td, &all_vertices, &graph.edges, refine_deadline)
}

/// The order the refined path picks its winner in: lowest width first, then
/// the smallest total bag size. Ties keep the earlier slot.
///
/// A caller that builds something from each decomposition and can price it
/// may rank by `(width, its own cost, total_bag_size)` instead; the two
/// orders pick different winners on the same input, on purpose.
pub fn refined_select_key(width: u32, total_bag_size: usize) -> (u32, usize) {
    (width, total_bag_size)
}
