//! Multilevel hypergraph partitioning (Karypis & Kumar, 1998); nparts=2 only.
//!
//! A hyperedge is charged to the cut once no matter how many of its vertices
//! straddle the split, where a graph in which the same set becomes a clique of
//! pairwise edges charges it once per cut pair.
//!
//! Four phases, one per submodule: `coarsen` contracts the hypergraph down,
//! `initial` partitions the coarsest level, `refine_fm` improves a partition by
//! single moves, `refine_flow` improves it by re-cutting a whole corridor at
//! once. `hg_multilevel_pass` drives them.
//!
//! Sibling of [`multilevel_bisect`](crate::multilevel_bisect), the graph
//! variant. The two share the containers and the pass bookkeeping in
//! `fm_common`; the gain models stay apart, and are not to be merged — each is
//! a perf-sensitive, benchmark-validated inner loop written against its own
//! notion of what a cut costs. Every other difference between the two is
//! recorded there as well, under "Where the two bisectors differ".

mod coarsen;
mod graph;
mod initial;
mod refine_flow;
mod refine_fm;
use coarsen::*;
use graph::*;
use initial::*;
use refine_flow::*;
use refine_fm::*;

use crate::fm_common::{index_split, tiny_bisection};
use crate::rng::{Xorshift64, bisector_stream};

const MIN_HG_COARSEN_SIZE: usize = 20;

/// Independent restarts of the whole multilevel sweep, each on its own RNG
/// stream, with the best hyperedge cut kept.
///
/// The effort budget enters as a square root, and the V-cycle count in
/// `multilevel_hg_bisect_once` takes the same square root, so raising the
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
    ((base as f64) * effort_scale.sqrt()).round() as usize
}

/// If `existing_part` is provided, uses partition-aware coarsening (V-cycle).
///
/// Returns 0/1 per vertex of `hg`. The projection down the levels is carried
/// incrementally, one majority vote per new level; the graph sibling replays
/// the whole chain from the original partition at every level instead, for the
/// reason recorded on its own `multilevel_pass`.
fn hg_multilevel_pass(
    hg: &Hypergraph,
    existing_part: Option<&[u8]>,
    rng: &mut Xorshift64,
    imbalance: f64,
) -> Vec<u8> {
    let n = hg.num_vertices;

    let mut levels: Vec<HgCoarseLevel> = Vec::new();
    let mut current = hg;

    let mut projected_part: Option<Vec<u8>> = existing_part.map(|p| p.to_vec());

    // Coarsening. `levels` ends up ordered finest-first, and `projected_part`
    // tracks the partition of `current` — a coarse vertex takes the side most
    // of its fine vertices are on, ties to side 0.
    loop {
        let coarse_part_ref = projected_part.as_deref();
        if let Some(level) =
            hg_coarsen_one_level(current, MIN_HG_COARSEN_SIZE, rng, coarse_part_ref)
        {
            if let Some(ref pp) = projected_part {
                let nc = level.hg.num_vertices;
                let mut cp = vec![0u8; nc];
                let mut count = vec![[0u32; 2]; nc];
                for (v, &m) in level.mapping.iter().enumerate() {
                    count[m as usize][pp[v] as usize] += 1;
                }
                for cv in 0..nc {
                    cp[cv] = if count[cv][1] > count[cv][0] { 1 } else { 0 };
                }
                projected_part = Some(cp);
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
        hg_initial_partition(current, rng, imbalance)
    };

    // Coarse hyperedges carry the summed weight of every fine hyperedge merged
    // into them, so a move here is worth many fine clauses.
    hg_fm_refine(current, &mut part, imbalance);

    // Uncoarsening. Each step hands every fine vertex its coarse vertex's side,
    // then refines with the freedom the finer hypergraph exposes; only the
    // finest level pays for the localized and flow passes on top.
    for (li, level) in levels.iter().enumerate().rev() {
        let fine_n = level.mapping.len();
        let mut fine_part = vec![0u8; fine_n];
        for v in 0..fine_n {
            fine_part[v] = part[level.mapping[v] as usize];
        }
        part = fine_part;

        let fine_hg = if li > 0 { &levels[li - 1].hg } else { hg };
        if li == 0 {
            hg_fm_refine_with_local(fine_hg, &mut part, imbalance);
        } else {
            hg_fm_refine(fine_hg, &mut part, imbalance);
        }
    }

    let count0 = part.iter().filter(|&&p| p == 0).count();
    if count0 == 0 || count0 == n {
        part = index_split(n);
    }

    part
}

fn multilevel_hg_bisect_once(
    hg: &Hypergraph,
    rng: &mut Xorshift64,
    imbalance: f64,
    effort_scale: f64,
) -> Vec<u8> {
    let mut part = hg_multilevel_pass(hg, None, rng, imbalance);

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
        let old_cut = hg_cut(hg, &part);
        let new_part = hg_multilevel_pass(hg, Some(&part), rng, imbalance);
        let new_cut = hg_cut(hg, &new_part);
        if new_cut < old_cut {
            part = new_part;
        } else {
            break;
        }
    }

    part
}

/// Multilevel 2-way bisection of a hypergraph: the best hyperedge cut over
/// several restarts. Returns one side per vertex, `0` or `1`, `num_vertices`
/// long; both sides are non-empty for any `num_vertices >= 2`.
///
/// `hyperedges` hold vertex ids in `0..num_vertices`, already deduplicated
/// within each hyperedge; `hewgt` is parallel to `hyperedges` and defaults to
/// all ones. `imbalance` bounds each side at `(0.5 + imbalance)` of the vertex
/// weight; `base_seed` selects the RNG streams the restarts run on, and
/// nothing here reads a clock. `effort_scale` multiplies the restart and
/// V-cycle counts, `1.0` being the calibrated baseline.
pub fn multilevel_hg_bisect(
    num_vertices: usize,
    hyperedges: &[Vec<u32>],
    hewgt: Option<&[u32]>,
    imbalance: f64,
    base_seed: u64,
    effort_scale: f64,
) -> Vec<u8> {
    if let Some(part) = tiny_bisection(num_vertices) {
        return part;
    }

    let hg = Hypergraph::from_hyperedges(num_vertices, hyperedges, hewgt);

    if hg.num_hyperedges() == 0 {
        return index_split(num_vertices);
    }

    let mut best_part = Vec::new();
    let mut best_cut = u32::MAX;

    // Best-of-N on the cut, the objective the caller asked for here; see "Where
    // the two bisectors differ" in `fm_common`.
    let restarts = num_hg_restarts(num_vertices, effort_scale);
    for restart in 0..restarts {
        let mut rng = bisector_stream(crate::bisect_seed::restart_seed(base_seed, restart));
        let part = multilevel_hg_bisect_once(&hg, &mut rng, imbalance, effort_scale);
        let cut = hg_cut(&hg, &part);
        if cut < best_cut {
            best_cut = cut;
            best_part = part;
        }
    }

    best_part
}
