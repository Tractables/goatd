//! Run several decomposition constructions and keep the candidates they
//! produce.
//!
//! The candidates themselves come from the elimination engine; the single-order
//! construction in [`crate::elimination::decompose`] does not go through
//! here.

use std::time::{Duration, Instant};

mod candidates;
mod config;

#[cfg(test)]
mod tests;

use crate::deadline::expired;
use crate::decomposition;
use crate::elimination::Order;
use crate::elimination::engine;
use crate::elimination::execution::ElimStop;
use crate::flowcutter::{Budget, decompose as flowcutter_decompose};
use crate::{Error, Graph, TreeDecomposition};
use candidates::{CandidateOutcome, CandidateSet};
use config::MIN_FLOWCUTTER_CANDIDATE_MS;

pub use config::PortfolioConfig;

/// Exit early if FlowCutter hasn't improved treewidth for this long. Caps
/// per-graph overhead where FlowCutter converges fast.
const FLOWCUTTER_CANDIDATE_PATIENCE: Duration = Duration::from_millis(500);
const FLOWCUTTER_CANDIDATE_ITERATIONS: u32 = 50;
const SAMPLE_SEED_OFFSET: u64 = 100;
const SAMPLE_SEED_STRIDE: u64 = 7919;
pub(crate) const SECOND_CANDIDATE_SEED_OFFSET: u64 = 42;

/// Residuals above this size run only min-degree candidates after the first;
/// the other orders can overrun a short portfolio budget at this scale.
const MAX_RESIDUAL_FOR_EXPENSIVE_ORDERS: usize = 10_000;

fn is_min_degree_variant(order: Order<'_>) -> bool {
    matches!(order, Order::MinDegree | Order::MinDegreeSampled { .. })
}

fn sample_seed(base_seed: u64, sample_index: u64) -> u64 {
    base_seed.wrapping_add(SAMPLE_SEED_OFFSET + sample_index.wrapping_mul(SAMPLE_SEED_STRIDE))
}

fn extra_sample<'a>(
    base_seed: u64,
    large_residual: bool,
    ordinary_runs: u64,
    diverse_runs: u64,
    index: u64,
    weights: &'a [u32],
) -> Option<(Order<'a>, u64)> {
    if large_residual {
        return (index < ordinary_runs).then_some((
            Order::MinDegreeSampled { weights },
            sample_seed(base_seed, index),
        ));
    }

    debug_assert!(diverse_runs <= config::DIVERSE_SAMPLING_RUNS);
    if index < diverse_runs {
        let order = match index {
            0 => Order::AdjacentFillSampled { weights },
            1 => Order::SparsestSubgraphSampled { weights },
            _ => unreachable!("there are only two diverse sampled orders"),
        };
        return Some((order, sample_seed(base_seed, 0)));
    }

    let ordinary_index = index - diverse_runs;
    (ordinary_index < ordinary_runs).then_some((
        Order::MinFillSampled { weights },
        sample_seed(base_seed, ordinary_index),
    ))
}

fn flowcutter_candidate(
    graph: &Graph,
    configured_budget: Duration,
    hard_deadline: Option<Instant>,
) -> Result<Option<TreeDecomposition>, Error> {
    let timeout = hard_deadline
        .map(crate::deadline::remaining)
        .unwrap_or(configured_budget)
        .min(configured_budget);
    // Skip windows too small to seed useful FlowCutter iterations; FFI overhead
    // alone eats tens of ms on small graphs.
    if timeout < Duration::from_millis(MIN_FLOWCUTTER_CANDIDATE_MS) {
        return Ok(None);
    }
    match flowcutter_decompose(
        graph,
        Budget::timed(
            timeout,
            Some(FLOWCUTTER_CANDIDATE_PATIENCE),
            FLOWCUTTER_CANDIDATE_ITERATIONS,
        ),
    ) {
        Ok(decomposition) => Ok(Some(decomposition)),
        // A timed backend run may end before it has a result. The elimination
        // candidates already make the portfolio complete, so this one can be
        // absent without changing the contract.
        Err(Error::NoDecomposition | Error::TooLarge(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Clone, Copy)]
enum EliminationPhase {
    Initial,
    ExtraSampling,
}

/// Initial candidates may use the complete two-stage window so the first one
/// can always return a decomposition. Extra samples stop at the soft deadline;
/// the rest of the hard window belongs to FlowCutter.
fn elimination_stop(
    phase: EliminationPhase,
    soft_deadline: Option<Instant>,
    hard_deadline: Option<Instant>,
    width_bound: Option<u32>,
) -> ElimStop {
    ElimStop {
        soft_deadline,
        hard_deadline: match phase {
            EliminationPhase::Initial => hard_deadline,
            EliminationPhase::ExtraSampling => soft_deadline,
        },
        width_bound,
    }
}

/// Builds a run's candidate list for a seed, given the weight vector.
type InitialOrderBuilder = for<'w> fn(u64, &'w [u32]) -> Vec<(Order<'w>, u64)>;

#[derive(Clone, Copy)]
enum CandidateRetention {
    All,
    BestOnly,
}

/// The five fixed candidates at the front of the standard portfolio, ordered
/// by the cost of one elimination step.
fn standard_orders(base_seed: u64, weights: &[u32]) -> Vec<(Order<'_>, u64)> {
    let second_seed = base_seed.wrapping_add(SECOND_CANDIDATE_SEED_OFFSET);
    vec![
        (Order::MinDegreeSampled { weights }, base_seed),
        (Order::NestedDissection, base_seed),
        (Order::MinFillSampled { weights }, base_seed),
        (Order::MinDegreeSampled { weights }, second_seed),
        (Order::NestedDissection, second_seed),
    ]
}

/// The fixed candidate at the front of the sampled-min-fill portfolio.
fn sampled_min_fill_orders(base_seed: u64, weights: &[u32]) -> Vec<(Order<'_>, u64)> {
    vec![(Order::MinFillSampled { weights }, base_seed)]
}

/// Run a portfolio: large-residual skips, then extra sampled orders with the
/// remaining budget, then the trailing FlowCutter candidate where configured.
///
/// At least one candidate always carries a decomposition: the first is exempt from both
/// between-candidate skips, runs with no `width_bound` (so it cannot abort on
/// width) and with deadline completion enabled (so a deadline stop still
/// yields a decomposition).
///
/// `weights` has one entry per vertex and is what the sampling orders draw tie
/// sets with; every candidate and every sampled order shares it.
fn run_portfolio(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    initial_orders: InitialOrderBuilder,
    config: PortfolioConfig,
    retention: CandidateRetention,
) -> Result<CandidateSet, crate::Error> {
    config::validate(config)?;
    let deadlines = crate::deadline::staged(
        crate::meter::now(),
        config.soft_budget,
        config.hard_budget,
        "portfolio",
    )?;
    let soft_deadline = deadlines.soft;
    let hard_deadline = deadlines.hard;
    let mut prebuilt = engine::prebuild(graph);
    let initial_orders = initial_orders(seed, weights);
    let large_residual = prebuilt.num_active() > MAX_RESIDUAL_FOR_EXPENSIVE_ORDERS;
    let mut candidates = match retention {
        CandidateRetention::All => CandidateSet::all(initial_orders.len() + 1),
        CandidateRetention::BestOnly => CandidateSet::best_only(),
    };

    // Set after any candidate reaches the hard deadline, or when it expires
    // between candidates. Later runs would stop at the same point.
    let mut hard_deadline_tripped = false;

    for (i, (order, candidate_seed)) in initial_orders.iter().copied().enumerate() {
        // Honour the deadline between orders (when set), but always run
        // order 0 so we return something even on huge graphs that would
        // otherwise time out inside the first order.
        if i > 0 && expired(soft_deadline) {
            break;
        }
        // On large residuals, only min-degree variants reliably complete;
        // nested dissection and min-fill can overrun a short budget.
        if i > 0 && large_residual && !is_min_degree_variant(order) {
            continue;
        }
        // Complete the residual only while no candidate has produced a usable
        // decomposition yet. Once one has, completing a later candidate's
        // residual would be wasted work: its wide decomposition would lose on
        // width and total bag size to the existing winner.
        let complete_on_deadline = candidates.is_empty();
        let run = engine::run_order_prebuilt(
            &mut prebuilt,
            engine::RunSpec {
                order,
                seed: candidate_seed,
                stop: elimination_stop(
                    EliminationPhase::Initial,
                    soft_deadline,
                    hard_deadline,
                    candidates.best_width(),
                ),
                complete_on_deadline,
            },
        );
        hard_deadline_tripped = match candidates.record_elimination(run) {
            CandidateOutcome::DeadlineReached => true,
            // Nothing usable from this candidate, but the portfolio is still inside
            // its budget.
            CandidateOutcome::WidthAborted => false,
            CandidateOutcome::Produced => expired(hard_deadline),
        };
        if hard_deadline_tripped {
            break;
        }
    }
    // Sampling phase: try additional seeds of the full-tie-set sampling
    // order with any remaining budget. Measured ≥79% of min-fill pops have
    // ≥2 tied candidates, so different seeds explore different
    // elimination orders and can lower width on small/medium graphs where the
    // base portfolio returns in tens of ms. Falls back to sampled min-degree on
    // large residuals, matching the main loop's skip rule. A started extra
    // sample stops at the soft deadline so it cannot consume the trailing
    // FlowCutter and output interval. On extended small/medium runs, two
    // diverse scores precede the complete ordinary min-fill seed sequence.
    let max_samples = config.sampling_runs;
    let diverse_samples = if large_residual {
        0
    } else {
        config.diverse_sampling_runs
    };
    let total_samples = max_samples.saturating_add(diverse_samples);
    let mut sample_index: u64 = 0;
    // Normally the soft deadline fires first; the portfolio hard-deadline
    // check also prevents another sample after an initial candidate used the
    // complete two-stage window.
    while sample_index < total_samples
        && !hard_deadline_tripped
        && !expired(soft_deadline)
        && !expired(hard_deadline)
    {
        let (sample_order, candidate_seed) = extra_sample(
            seed,
            large_residual,
            max_samples,
            diverse_samples,
            sample_index,
            weights,
        )
        .expect("sample index is below the configured total");
        // Extra sampling only runs after the fixed candidates, so at least one prior
        // candidate won, so deadline completion is unnecessary here.
        let run = engine::run_order_prebuilt(
            &mut prebuilt,
            engine::RunSpec {
                order: sample_order,
                seed: candidate_seed,
                stop: elimination_stop(
                    EliminationPhase::ExtraSampling,
                    soft_deadline,
                    hard_deadline,
                    candidates.best_width(),
                ),
                complete_on_deadline: false,
            },
        );
        match candidates.record_elimination(run) {
            // No time left for more sampled orders.
            CandidateOutcome::DeadlineReached => break,
            // A width-aborted seed keeps sampling: another seed
            // explores a different elimination order.
            CandidateOutcome::Produced | CandidateOutcome::WidthAborted => sample_index += 1,
        }
    }
    // Runs vanilla FlowCutter once as a final portfolio candidate. Placed after
    // the extra-sampling loop so it runs in the remaining hard-deadline
    // margin without starving sampling — under a typical soft-budget
    // contract, `hard_deadline` = 2×`soft_deadline`, leaving up to
    // `soft_deadline` of slack here. FlowCutter already returns a complete
    // decomposition, so no separator-refinement pass is applied to it.
    if let Some(configured_budget) = config
        .flowcutter_budget
        .filter(|_| !hard_deadline_tripped && !expired(hard_deadline))
        && let Some(decomposition) = flowcutter_candidate(graph, configured_budget, hard_deadline)?
    {
        candidates.push(decomposition);
    }
    Ok(candidates)
}

/// Run one sampled min-fill order, then up to
/// the configured number of further seeds, then an optional trailing
/// FlowCutter candidate — and return every decomposition produced, in candidate
/// order. Never empty: the first candidate always produces one.
///
/// The caller picks among them, commonly by width and then total bag size.
///
/// # Errors
///
/// Returns an error for a weight count that differs from the graph vertex
/// count, an invalid deadline or FlowCutter budget, or an invalid FlowCutter
/// result.
pub fn sampled_min_fill_candidates(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
) -> Result<Vec<TreeDecomposition>, crate::Error> {
    validate_weights(graph, weights)?;
    Ok(run_portfolio(
        graph,
        weights,
        seed,
        sampled_min_fill_orders,
        config,
        CandidateRetention::All,
    )?
    .into_decompositions())
}

fn standard_candidate_set(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
    retention: CandidateRetention,
) -> Result<CandidateSet, crate::Error> {
    validate_weights(graph, weights)?;
    run_portfolio(graph, weights, seed, standard_orders, config, retention)
}

/// Run the standard portfolio and return every decomposition it produced,
/// sorted ascending by width and total bag size, with ties kept in candidate order
/// (a stable sort), so the first is the portfolio's winner. Never empty.
///
/// # Errors
///
/// Returns an error for a weight count that differs from the graph vertex
/// count, an invalid deadline or FlowCutter budget, or an invalid FlowCutter
/// result.
pub fn candidates(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
) -> Result<Vec<TreeDecomposition>, crate::Error> {
    let mut decompositions =
        standard_candidate_set(graph, weights, seed, config, CandidateRetention::All)?
            .into_decompositions();
    decompositions.sort_by_key(TreeDecomposition::quality_key);
    Ok(decompositions)
}

/// Return the standard portfolio's best candidate by width, then total bag
/// size. Bags contained in an adjacent bag are contracted before return.
///
/// # Errors
///
/// Returns the same configuration and weight errors as [`candidates`].
pub fn decompose(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
) -> Result<TreeDecomposition, crate::Error> {
    Ok(
        standard_candidate_set(graph, weights, seed, config, CandidateRetention::BestOnly)?
            .into_decompositions()
            .into_iter()
            .next()
            .expect("first candidate always produces a decomposition"),
    )
}

/// The standard portfolio's winner, refined by FlowCutter cuts
/// ([`refine_with_flowcutter`](crate::decomposition::refine_with_flowcutter)).
///
/// `refinement_budget` bounds the refinement pass. Both halves are anytime:
/// the portfolio keeps the best decomposition found so far, and a skipped
/// refinement returns it unchanged.
///
/// # Errors
///
/// Returns the same errors as [`decompose`] and
/// [`refine_with_flowcutter`](crate::decomposition::refine_with_flowcutter).
pub fn decompose_and_refine(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
    refinement_budget: Option<Duration>,
) -> Result<TreeDecomposition, crate::Error> {
    let td = decompose(graph, weights, seed, config)?;
    decomposition::refine_with_flowcutter(td, graph, refinement_budget)
}

fn validate_weights(graph: &Graph, weights: &[u32]) -> Result<(), crate::Error> {
    if weights.len() != graph.num_vertices as usize {
        return Err(crate::Error::InvalidInput(format!(
            "portfolio has {} weights for {} vertices",
            weights.len(),
            graph.num_vertices
        )));
    }
    Ok(())
}
