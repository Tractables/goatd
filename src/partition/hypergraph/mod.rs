//! Multilevel hypergraph bisection (Karypis & Kumar, 1998); two parts only.
//!
//! A hyperedge is charged to the cut once no matter how many of its vertices
//! straddle the split, where a graph in which the same set becomes a clique of
//! pairwise edges charges it once per cut pair.
//!
//! Four phases, one per submodule: `coarsen` contracts the hypergraph down,
//! `initial` partitions the coarsest level, `refine_fm` improves a partition by
//! single moves, `refine_flow` improves it by re-cutting a whole corridor at
//! once. `multilevel_pass` drives them.
//!
//! Sibling of
//! [`multilevel_graph_bisect`](crate::partition::multilevel_graph_bisect).
//! The two share pass bookkeeping. Their gain calculations stay separate
//! because edge cuts and hyperedge cuts change differently when a vertex
//! moves. The other differences are listed in the shared bookkeeping module.

mod coarsen;
mod initial;
mod model;
mod refine_flow;
mod refine_fm;

#[cfg(test)]
mod tests;

use coarsen::{CoarseningLevel, coarsen_one_level};
use initial::{hyperedge_cut, initial_partition};
use refine_flow::refine_finest_level;
use refine_fm::refine_level;

use crate::Error;
use crate::partition::Bisection;
use crate::partition::common::{
    index_split, lift_to_fine, project_to_coarse, repair_bisection, tiny_bisection,
    validate_max_imbalance,
};
use crate::rng::{Xorshift64, bisector_stream, restart_seed};

const MIN_HG_COARSEN_SIZE: usize = 20;
const MAX_EFFORT: f64 = 100.0;

pub use model::Hypergraph;

/// Controls one multilevel hypergraph bisection.
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub struct HypergraphBisectionConfig {
    max_imbalance: f64,
    seed: u64,
    effort: f64,
}

impl HypergraphBisectionConfig {
    /// Configure a bisection at the baseline effort. `max_imbalance` is the
    /// allowed deviation from a half-and-half split, in `0.0..=0.5`; `seed`
    /// selects the deterministic RNG streams.
    pub fn new(max_imbalance: f64, seed: u64) -> Self {
        Self {
            max_imbalance,
            seed,
            effort: 1.0,
        }
    }

    /// Scale the number of restarts and V-cycles. The baseline is `1.0`; valid
    /// values are positive and at most 100.
    pub fn with_effort(mut self, effort: f64) -> Self {
        self.effort = effort;
        self
    }
}

/// Independent restarts of the whole multilevel sweep, each on its own RNG
/// stream, with the best hyperedge cut kept.
///
/// The effort budget enters as a square root, and the V-cycle count in
/// `multilevel_hypergraph_bisect_once` takes the same square root, so raising the
/// budget splits between more restarts and more refinement of each rather than
/// multiplying into either.
fn num_hg_restarts(n: usize, effort_scale: f64) -> usize {
    let base = if n >= 400 {
        6
    } else if n >= 100 {
        4
    } else {
        2
    };
    (((base as f64) * effort_scale.sqrt()).round() as usize).max(1)
}

/// If `existing_part` is provided, uses partition-aware coarsening (V-cycle).
///
/// Returns 0/1 per vertex of `hg`. The projection down the levels is carried
/// incrementally, one majority vote per new level; the graph sibling replays
/// the whole chain from the original partition at every level instead, for the
/// reason recorded on its own `multilevel_pass`.
fn multilevel_pass(
    hg: &Hypergraph,
    existing_part: Option<&[u8]>,
    rng: &mut Xorshift64,
    imbalance: f64,
) -> Vec<u8> {
    let n = hg.num_vertices;

    let mut levels: Vec<CoarseningLevel> = Vec::new();
    let mut current = hg;

    let mut projected_part: Option<Vec<u8>> = existing_part.map(|p| p.to_vec());
    let mut projection_counts = Vec::with_capacity(n);
    let mut projection = Vec::with_capacity(n);

    // Coarsening. `levels` ends up ordered finest-first, and `projected_part`
    // tracks the partition of `current` — a coarse vertex takes the side most
    // of its fine vertices are on, ties to side 0.
    loop {
        let coarse_part_ref = projected_part.as_deref();
        if let Some(level) = coarsen_one_level(current, MIN_HG_COARSEN_SIZE, rng, coarse_part_ref) {
            if let Some(ref mut pp) = projected_part {
                let nc = level.hg.num_vertices;
                project_to_coarse(
                    pp,
                    &level.mapping,
                    nc,
                    &mut projection_counts,
                    &mut projection,
                );
                std::mem::swap(pp, &mut projection);
            }
            levels.push(level);
            current = &levels.last().unwrap().hg;
        } else {
            break;
        }
    }

    // Coarsest level: either the caller's partition projected the whole way
    // down, or a fresh one grown here. This is the only point in the sweep
    // where a partition is created rather than improved.
    let mut part = if let Some(pp) = projected_part {
        pp
    } else {
        initial_partition(current, rng, imbalance)
    };

    // Coarse hyperedges carry the summed weight of every fine hyperedge merged
    // into them, so a move here can be worth many fine hyperedges.
    refine_level(current, &mut part, imbalance);

    // Uncoarsening. Each step hands every fine vertex its coarse vertex's side,
    // then refines with the freedom the finer hypergraph exposes; only the
    // finest level pays for the localized and flow passes on top.
    for (li, level) in levels.iter().enumerate().rev() {
        lift_to_fine(&part, &level.mapping, &mut projection);
        std::mem::swap(&mut part, &mut projection);

        let fine_hg = if li > 0 { &levels[li - 1].hg } else { hg };
        if li == 0 {
            refine_finest_level(fine_hg, &mut part, imbalance);
        } else {
            refine_level(fine_hg, &mut part, imbalance);
        }
    }

    repair_bisection(part, imbalance)
}

fn multilevel_bisect_once(
    hg: &Hypergraph,
    rng: &mut Xorshift64,
    imbalance: f64,
    effort_scale: f64,
) -> Vec<u8> {
    let mut part = multilevel_pass(hg, None, rng, imbalance);

    // Arbitrary tuned thresholds: more V-cycles for larger hypergraphs, where
    // quality matters more.
    let vc_base = if hg.num_vertices >= 400 {
        4
    } else if hg.num_vertices >= 100 {
        2
    } else {
        1
    };
    let num_vcycles = (vc_base as f64 * effort_scale.sqrt()).round() as usize;
    // The first cycle that fails to improve ends the loop, so `num_vcycles` is
    // a ceiling rather than a count.
    for _ in 0..num_vcycles {
        let old_cut = hyperedge_cut(hg, &part);
        let new_part = multilevel_pass(hg, Some(&part), rng, imbalance);
        let new_cut = hyperedge_cut(hg, &new_part);
        if new_cut < old_cut {
            part = new_part;
        } else {
            break;
        }
    }

    part
}

/// Multilevel 2-way bisection of a hypergraph: the best hyperedge cut over
/// several restarts. The result contains both sides when there are at least
/// two vertices.
///
/// `config.max_imbalance` bounds each side at
/// `ceil((0.5 + max_imbalance) * num_vertices)`. `config.seed` selects the RNG
/// streams; nothing here reads a clock. `config.effort` scales restart and
/// V-cycle counts, with `1.0` as the baseline.
///
/// # Errors
///
/// Returns an error when the imbalance or effort is outside its documented
/// range.
pub fn multilevel_hypergraph_bisect(
    hg: &Hypergraph,
    config: HypergraphBisectionConfig,
) -> Result<Bisection, Error> {
    validate_max_imbalance(config.max_imbalance, "hypergraph-bisection")?;
    if !config.effort.is_finite() || config.effort <= 0.0 || config.effort > MAX_EFFORT {
        return Err(Error::InvalidInput(format!(
            "hypergraph-bisection effort must be greater than 0.0 and at most {MAX_EFFORT}, got {}",
            config.effort,
        )));
    }
    let num_vertices = hg.num_vertices;
    if let Some(part) = tiny_bisection(num_vertices) {
        return Ok(Bisection::new(part));
    }

    if hg.num_hyperedges() == 0 {
        return Ok(Bisection::new(index_split(num_vertices)));
    }

    let mut best_part = Vec::new();
    let mut best_cut = u32::MAX;

    // Best-of-N on the cut, the objective the caller asked for here; see "Where
    // the two bisectors differ" in the shared partition bookkeeping.
    let restarts = num_hg_restarts(num_vertices, config.effort);
    for restart in 0..restarts {
        let mut rng = bisector_stream(restart_seed(config.seed, restart));
        let part = multilevel_bisect_once(hg, &mut rng, config.max_imbalance, config.effort);
        let candidate_cut = hyperedge_cut(hg, &part);
        if candidate_cut < best_cut {
            best_cut = candidate_cut;
            best_part = part;
        }
    }

    Ok(Bisection::new(best_part))
}
