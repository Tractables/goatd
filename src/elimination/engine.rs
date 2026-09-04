//! Shared execution machinery for elimination orders and portfolios.

use std::collections::VecDeque;
use std::time::Instant;

use super::Order;
use super::build_td::build_td_from_ranked_bags;
use super::execution::{self, Cutoff, ElimExit, ElimSteps};
use super::graph::EliminationGraph;
use super::greedy::{
    self, SampleDraw, eliminate_min_degree, eliminate_min_fill, eliminate_sampled_fill_degree,
    eliminate_sampled_min_degree, eliminate_sampled_min_fill,
};
use super::nested_dissection::eliminate_nested_dissection;
use super::preprocess::{Reduced, preprocess};
use crate::TreeDecomposition;
use crate::rng::{SEED_OFFSET, Xorshift64};

use super::execution::ElimStop;

/// The usable result of one `(order, seed)` run.
///
/// Partial bags never cross this boundary as a [`TreeDecomposition`]. A
/// deadline can either complete them with a residual path construction or
/// report `DeadlineAborted`, depending on the caller's
/// `complete_on_deadline` setting. Both carry which cutoff stopped the run,
/// because a run stopped at the soft cutoff leaves the caller's remaining
/// hard-deadline time to spend and one stopped at the hard cutoff does not.
pub(crate) enum OrderRun {
    Completed(TreeDecomposition),
    CompletedAtDeadline(Cutoff, TreeDecomposition),
    DeadlineAborted(Cutoff),
    WidthAborted,
}

/// Graph + preprocessing result, shared across every order in a portfolio.
/// Graph construction and preprocessing are deterministic, so every order in
/// a portfolio can clone this result rather than repeat that work.
pub(crate) struct Prebuilt {
    reduced: Reduced,
    /// Connected components of the preprocessed residual, reused by every
    /// portfolio candidate.
    components: Vec<Vec<u32>>,
    /// Initial fill count of each residual vertex. Computed on the first
    /// sampled min-fill run, then reused by later seeds.
    initial_fill: Option<Vec<u64>>,
}

/// Build the elimination representation and preprocess it once for reuse
/// across every order in a portfolio.
pub(crate) fn prebuild(input: &crate::Graph, soft_deadline: Option<Instant>) -> Prebuilt {
    let graph = EliminationGraph::from_edges(input.num_vertices, &input.edges);
    finish_prebuild(preprocess(graph, soft_deadline))
}

/// Build the elimination representation without applying reduction rules.
pub(crate) fn prebuild_original(input: &crate::Graph) -> Prebuilt {
    finish_prebuild(Reduced {
        graph: EliminationGraph::from_edges(input.num_vertices, &input.edges),
        prefix: ElimSteps::default(),
    })
}

fn finish_prebuild(reduced: Reduced) -> Prebuilt {
    let components = find_connected_components(&reduced.graph);
    Prebuilt {
        reduced,
        components,
        initial_fill: None,
    }
}

impl Prebuilt {
    /// Number of active vertices in the preprocessed residual graph.
    pub(crate) fn num_active(&self) -> usize {
        self.reduced.graph.num_active
    }
}

/// One candidate run: which core to use, its RNG stream, and its cutoffs. It
/// travels unchanged from the portfolio down to the elimination
/// core, so a path that needs a variation — the per-component solver, with its
/// own seed and its own slice of the weights — re-points those fields and
/// passes the rest straight through.
///
#[derive(Clone, Copy)]
pub(crate) struct RunSpec<'a> {
    /// Which algorithmic core produces the elimination order, carrying the
    /// sampling weight if it is one that reads one.
    pub(crate) order: Order<'a>,
    /// Selects the RNG stream for salt and tie-set sampling.
    pub(crate) seed: u64,
    /// How far above the smallest score a sampled order's tie set reaches, in
    /// the units of the order's own score. 0 is the exact minimum; the
    /// non-sampling cores ignore it.
    pub(crate) sample_band: u64,
    /// Break min-degree ties by the order in which scores were updated.
    pub(crate) update_order_ties: bool,
    /// When the elimination must stop — handed to the core whole.
    pub(crate) stop: ElimStop,
    /// Whether a deadline stop must still leave a complete TD behind.
    pub(crate) complete_on_deadline: bool,
    /// An instant the run's setup honours: with one set, the fill counts a
    /// fill-based order seeds its buckets from are computed with the clock
    /// read between vertices, and a run whose setup reaches the instant
    /// reports the hard cutoff without eliminating. Without one the setup
    /// runs to completion, which is what every run did before the field.
    pub(crate) setup_deadline: Option<Instant>,
}

/// Run a single spec using a preprocessed graph. The first sampled min-fill
/// run populates its reusable fill-count cache; every run clones the reduced
/// graph before mutating it.
pub(crate) fn run_order_prebuilt(prebuilt: &mut Prebuilt, spec: RunSpec<'_>) -> OrderRun {
    if spec.order.uses_initial_fill_cache() && prebuilt.initial_fill.is_none() {
        let graph = &prebuilt.reduced.graph;
        let fill = if let Some(deadline) = spec.setup_deadline {
            // The count is quadratic in a vertex's degree, and on a graph of a
            // million edges the whole pass outlasts a short cutoff, so it is
            // paced like an elimination: the clock is read between vertices
            // and a pass that reaches the cutoff leaves no cache behind.
            let mut scratch = greedy::FillScratch::new(graph.len());
            let mut pacer = execution::DeadlinePacer::new();
            let mut counts = Vec::with_capacity(graph.len());
            for vertex in 0..graph.len() {
                if graph.active[vertex] {
                    if pacer.due() && crate::deadline::expired(Some(deadline)) {
                        return OrderRun::DeadlineAborted(Cutoff::Hard);
                    }
                    counts.push(scratch.fill_count_of(graph, vertex as u32));
                } else {
                    counts.push(0);
                }
            }
            counts
        } else if graph.bitset_words > 0 {
            (0..graph.len())
                .map(|vertex| {
                    if graph.active[vertex] {
                        graph.fill_count_of_bs(vertex as u32)
                    } else {
                        0
                    }
                })
                .collect()
        } else {
            greedy::compute_initial_fill(graph)
        };
        prebuilt.initial_fill = Some(fill);
    }
    let clone_bitset_only =
        prebuilt.components.len() == 1 && prebuilt.reduced.graph.bitset_words > 0;
    let reduced = if clone_bitset_only {
        Reduced {
            graph: prebuilt.reduced.graph.clone_bitset_only(),
            prefix: prebuilt.reduced.prefix.clone(),
        }
    } else {
        prebuilt.reduced.clone()
    };
    run_order_on_reduced(
        reduced,
        &prebuilt.components,
        prebuilt.initial_fill.as_deref(),
        spec,
    )
}

/// BFS connected-component finder on the active residual. Returns one Vec<u32>
/// per component; each vec contains the original vertex ids in visit order.
/// Uses `collect_live_nbrs_into` (bitset-aware): after `preprocess` runs
/// `eliminate_with_nbrs_bs`, `adj` is no longer maintained and reading it
/// directly would produce stale neighbour lists (missing fill edges, extra
/// eliminated entries) and partition the graph incorrectly.
pub(super) fn find_connected_components(graph: &EliminationGraph) -> Vec<Vec<u32>> {
    let n = graph.len();
    let mut visited = vec![false; n];
    let mut components: Vec<Vec<u32>> = Vec::new();
    let mut nbrs_buf: Vec<u32> = Vec::new();
    for start in 0..n {
        if !graph.active[start] || visited[start] {
            continue;
        }
        let mut comp: Vec<u32> = Vec::new();
        let mut queue = VecDeque::new();
        visited[start] = true;
        queue.push_back(start as u32);
        while let Some(v) = queue.pop_front() {
            comp.push(v);
            nbrs_buf.clear();
            graph.collect_live_nbrs_into(v, &mut nbrs_buf);
            for &u in &nbrs_buf {
                let ui = u as usize;
                if graph.active[ui] && !visited[ui] {
                    visited[ui] = true;
                    queue.push_back(u);
                }
            }
        }
        components.push(comp);
    }
    components
}

/// Run elimination on an already-preprocessed residual and return the raw bags
/// and rank_pairs (in the graph's own vertex indices).
///
/// The one place that maps an [`Order`] onto an elimination core — both the
/// whole-residual and per-component callers go through it. Neither gets a
/// `OrderRun` back — the whole-residual caller runs `finalize` on the raw
/// output, the per-component caller first translates component-local indices
/// back to originals and concatenates into the global flat list that
/// `build_td_from_ranked_bags` consumes.
///
/// `initial_fill` is the caller's cached per-vertex fill count for this exact
/// graph; the min-fill sampling cores are the only ones that read it. A
/// component run remaps the counts into its local numbering.
fn run_elimination_raw(
    reduced: Reduced,
    salt: &[u32],
    initial_fill: Option<&[u64]>,
    spec: RunSpec<'_>,
) -> (ElimSteps, ElimExit, Vec<u32>) {
    let mut steps = reduced.prefix;
    let mut g = reduced.graph;

    let exit = match spec.order {
        Order::MinFill => eliminate_min_fill(&mut g, salt, steps.sink(), spec.stop),
        Order::MinDegree => eliminate_min_degree(
            &mut g,
            salt,
            spec.update_order_ties,
            steps.sink(),
            spec.stop,
        ),
        Order::MinFillSampled { weights } => eliminate_sampled_min_fill(
            &mut g,
            SampleDraw {
                weights,
                band: spec.sample_band,
            },
            spec.seed,
            steps.sink(),
            ElimStop {
                soft_deadline: None,
                ..spec.stop
            },
            initial_fill,
        ),
        Order::MinDegreeSampled { weights } => eliminate_sampled_min_degree(
            &mut g,
            SampleDraw {
                weights,
                band: spec.sample_band,
            },
            spec.seed,
            steps.sink(),
            ElimStop {
                soft_deadline: None,
                ..spec.stop
            },
        ),
        Order::FillDegreeSampled {
            weights,
            degree_coefficient,
        } => eliminate_sampled_fill_degree(
            &mut g,
            SampleDraw {
                weights,
                band: spec.sample_band,
            },
            spec.seed,
            steps.sink(),
            ElimStop {
                soft_deadline: None,
                ..spec.stop
            },
            initial_fill,
            degree_coefficient,
        ),
        Order::NestedDissection => {
            eliminate_nested_dissection(&mut g, salt, spec.seed, steps.sink(), spec.stop)
        }
    };

    let residual = if spec.complete_on_deadline && matches!(exit, ElimExit::DeadlineReached(_)) {
        execution::active_vertices(&g)
    } else {
        Vec::new()
    };
    (steps, exit, residual)
}

/// Solve the preprocessed residual one connected component at a time, then
/// stitch all bags (prefix + per-component) into a single flat list and run
/// `build_td_from_ranked_bags`. Components are vertex-disjoint (guaranteed by
/// connectivity), so each vertex is eliminated exactly once and `global_rank`
/// is written without conflicts.
///
/// Key invariant: prefix bags use original vertex ids; per-component bags use
/// component-local ids that must be translated back to originals before
/// appending to `all_bags`.
fn run_order_per_component(
    reduced: Reduced,
    components: &[Vec<u32>],
    salt: &[u32],
    initial_fill: Option<&[u64]>,
    spec: RunSpec<'_>,
) -> OrderRun {
    let n = reduced.graph.len();
    let mut all_bags: Vec<Vec<u32>> = reduced.prefix.bags;
    let mut global_rank: Vec<u32> = vec![u32::MAX; n];

    // Prefix ranks: (vertex, step) where step == bag index.
    for &(v, s) in &reduced.prefix.rank_pairs {
        global_rank[v as usize] = s as u32;
    }

    let mut nbrs_buf: Vec<u32> = Vec::new();
    let mut local_of = vec![u32::MAX; n];
    // Components after a soft-cutoff stop run against the hard deadline alone,
    // so this changes once and applies to every later component.
    let mut stop = spec.stop;
    let mut soft_cutoff_passed = false;
    for (comp_idx, comp) in components.iter().enumerate() {
        let comp_n = comp.len() as u32;
        for (i, &v) in comp.iter().enumerate() {
            local_of[v as usize] = i as u32;
        }
        // Extract component edges in local indexing (bitset-aware: adj may
        // be stale after preprocess).
        let mut comp_edges: Vec<(u32, u32)> = Vec::new();
        for &v in comp {
            nbrs_buf.clear();
            reduced.graph.collect_live_nbrs_into(v, &mut nbrs_buf);
            for &u in &nbrs_buf {
                if u > v {
                    comp_edges.push((local_of[v as usize], local_of[u as usize]));
                }
            }
        }

        // Global preprocessing already reached a fixed point. Restricting the
        // residual to one connected component changes no neighborhood, so a
        // second preprocessing pass could not fire another rule.
        let sub_reduced = Reduced {
            graph: EliminationGraph::from_edges(comp_n, &comp_edges),
            prefix: ElimSteps::default(),
        };
        let sub_initial_fill: Option<Vec<u64>> =
            initial_fill.map(|fill| comp.iter().map(|&vertex| fill[vertex as usize]).collect());

        let sub_salt: Vec<u32> = comp.iter().map(|&v| salt[v as usize]).collect();
        // This component is re-indexed from 0, so a sampling core's weight is
        // re-indexed with it; a deterministic core has none to re-index.
        let sub_weight: Option<Vec<u32>> = spec
            .order
            .tie_weights()
            .map(|w| comp.iter().map(|&v| w[v as usize]).collect());
        // Vary seed per component so sampling orders get independent randomness.
        let sub_spec = RunSpec {
            seed: spec.seed.wrapping_add(comp_idx as u64 + 1),
            order: match sub_weight.as_deref() {
                Some(weights) => spec.order.with_tie_weights(weights),
                None => spec.order,
            },
            stop,
            ..spec
        };

        let (comp_steps, comp_exit, comp_residual) = run_elimination_raw(
            sub_reduced,
            &sub_salt,
            sub_initial_fill.as_deref(),
            sub_spec,
        );
        match comp_exit {
            ElimExit::WidthLimitExceeded => return OrderRun::WidthAborted,
            ElimExit::DeadlineReached(cutoff) if !spec.complete_on_deadline => {
                return OrderRun::DeadlineAborted(cutoff);
            }
            ElimExit::Complete | ElimExit::DeadlineReached(_) => {
                comp_steps.append_reindexed(comp, &mut all_bags, &mut global_rank);
            }
        }

        if matches!(comp_exit, ElimExit::DeadlineReached(_)) {
            append_residual_bag(
                comp_residual
                    .into_iter()
                    .map(|vertex| comp[vertex as usize]),
                &mut all_bags,
                &mut global_rank,
            );
            // The soft cutoff is the construction budget, not the end of the
            // run: the components after this one still have the hard deadline
            // to be decomposed in, and one bag each is what they fall back to
            // when it passes. Their orders run against the hard deadline
            // alone, since the soft one has gone.
            if comp_exit == ElimExit::DeadlineReached(Cutoff::Soft) {
                soft_cutoff_passed = true;
                stop = ElimStop {
                    soft_deadline: None,
                    ..stop
                };
                continue;
            }
            // After the hard cutoff there is no time to start another order.
            for remaining in &components[(comp_idx + 1)..] {
                append_residual_bag(remaining.iter().copied(), &mut all_bags, &mut global_rank);
            }
            return finish(all_bags, global_rank, comp_exit, true);
        }
    }

    // A run that bagged one component's residual on the way through stopped at
    // a deadline, whatever the components after it managed.
    let exit = if soft_cutoff_passed {
        ElimExit::DeadlineReached(Cutoff::Soft)
    } else {
        ElimExit::Complete
    };
    finish(all_bags, global_rank, exit, spec.complete_on_deadline)
}

pub(super) fn run_order_on_reduced(
    reduced: Reduced,
    components: &[Vec<u32>],
    initial_fill: Option<&[u64]>,
    spec: RunSpec<'_>,
) -> OrderRun {
    let n = reduced.graph.len();
    // `+ SEED_OFFSET` avoids xorshift64's zero fixed point. The update-order
    // min-degree variant does not read the salt, but keeping allocation here
    // avoids another representation in component remapping.
    let mut rng = Xorshift64::from_state(spec.seed.wrapping_add(SEED_OFFSET));
    let salt: Vec<u32> = (0..n).map(|_| rng.next_u32()).collect();

    // Solve each connected component independently. Components arise
    // naturally after preprocessing removes low-degree vertices.
    if components.len() > 1 {
        return run_order_per_component(reduced, components, &salt, initial_fill, spec);
    }

    // Only the whole-residual path checks this: the per-component path
    // re-indexes each component and remaps the weight alongside, so its
    // lengths are its own business.
    if let Some(weights) = spec.order.tie_weights() {
        assert_eq!(
            weights.len(),
            n,
            "sampling weight count must match vertex count"
        );
    }

    let (steps, exit, residual) = run_elimination_raw(reduced, &salt, initial_fill, spec);
    finalize(steps, n, exit, spec.complete_on_deadline, residual)
}

fn append_residual_bag(
    vertices: impl IntoIterator<Item = u32>,
    bags: &mut Vec<Vec<u32>>,
    rank: &mut [u32],
) {
    let vertices: Vec<u32> = vertices.into_iter().collect();
    if vertices.is_empty() {
        return;
    }
    let bag_index = bags.len() as u32;
    for &vertex in &vertices {
        rank[vertex as usize] = bag_index;
    }
    bags.push(vertices);
}

fn finalize(
    mut steps: ElimSteps,
    n: usize,
    exit: ElimExit,
    complete_on_deadline: bool,
    residual: Vec<u32>,
) -> OrderRun {
    let mut rank = vec![u32::MAX; n];
    for (v, r) in steps.rank_pairs {
        rank[v as usize] = r as u32;
    }
    append_residual_bag(residual, &mut steps.bags, &mut rank);
    finish(steps.bags, rank, exit, complete_on_deadline)
}

fn finish(
    bags: Vec<Vec<u32>>,
    rank: Vec<u32>,
    exit: ElimExit,
    complete_on_deadline: bool,
) -> OrderRun {
    match exit {
        ElimExit::Complete => OrderRun::Completed(build_td_from_ranked_bags(bags, &rank)),
        ElimExit::DeadlineReached(cutoff) if complete_on_deadline => {
            OrderRun::CompletedAtDeadline(cutoff, build_td_from_ranked_bags(bags, &rank))
        }
        ElimExit::DeadlineReached(cutoff) => OrderRun::DeadlineAborted(cutoff),
        ElimExit::WidthLimitExceeded => OrderRun::WidthAborted,
    }
}
