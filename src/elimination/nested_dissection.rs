//! Nested-dissection elimination ordering via multilevel graph bisection.
//!
//! Recurse:
//!   1. Run multilevel bisection to partition vertices into two sides.
//!   2. Extract a minimum vertex cover of the bipartite cross-edge graph via
//!      König-Egerváry — this is the smallest vertex separator derivable from
//!      the given partition.
//!   3. Recurse on each side of `V \ separator`.
//!   4. Concatenate `order(A) ++ order(B) ++ order(separator)`.
//!
//! Base case (small subgraph, or [`MAX_RECURSION_DEPTH`] levels down): run
//! min-fill on the induced subgraph. The returned vector is a full elimination
//! order over `active` in global IDs.

use std::time::Instant;

use rustc_hash::FxHashSet;

use super::execution::{
    Cutoff, DeadlinePacer, ElimExit, ElimSink, ElimSteps, ElimStop, exceeds_width_bound,
};
use super::graph::EliminationGraph;
use super::greedy::eliminate_min_fill;
use super::vertex_cover_separator;
use crate::deadline::expired;
use crate::graph::index_by_vertex;
use crate::partition::{GraphBisectionConfig, multilevel_graph_bisect};

/// Default cutoff: once the induced subgraph has ≤ this many vertices, fall
/// back to local min-fill. Keeps recursion cost bounded while still letting
/// min-fill handle the dense tail where it does best.
const DEFAULT_BASE_CASE_SIZE: usize = 32;
const DEFAULT_MAX_IMBALANCE: f64 = 0.2;

/// Recursion limit for a sequence of poor bisections. At this depth the
/// remaining subgraph is finished with min-fill.
const MAX_RECURSION_DEPTH: u32 = 64;

/// What a whole nested-dissection recursion runs under. Every field is the
/// same at every level, so they travel as one reference rather than being
/// re-threaded through each call.
pub(super) struct NestedDissectionParams<'a> {
    /// `salt[v]` is the RNG-salt for global vertex `v`, used by the base-case
    /// min-fill for tie-breaking.
    pub(super) salt: &'a [u32],
    /// Subgraph size at or below which a level stops splitting and runs
    /// min-fill instead.
    pub(super) base_case_size: usize,
    /// Balance tolerance handed to the bisector at every level.
    pub(super) max_imbalance: f64,
    /// Hard cutoff, checked at each recursion level. Once reached, the current
    /// vertices are returned in salt order as a complete fallback order.
    pub(super) hard_deadline: Option<Instant>,
    /// The portfolio candidate's seed, carried unchanged down the whole
    /// recursion to `multilevel_graph_bisect`. Without it the standard
    /// portfolio's two `NestedDissection` candidates produce identical
    /// separators.
    pub(super) base_seed: u64,
}

/// Build and apply a nested-dissection order to the active residual.
pub(super) fn eliminate_nested_dissection(
    graph: &mut EliminationGraph,
    salt: &[u32],
    seed: u64,
    mut sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    let active: Vec<u32> = (0..graph.len() as u32)
        .filter(|&vertex| graph.active[vertex as usize])
        .collect();
    // Preprocessing may leave the list representation stale after switching
    // to bitsets, so read the residual through the representation-neutral
    // accessor.
    let mut neighbours = Vec::new();
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for &vertex in &active {
        neighbours.clear();
        graph.collect_live_nbrs_into(vertex, &mut neighbours);
        edges.extend(
            neighbours
                .iter()
                .copied()
                .filter(|&neighbour| neighbour > vertex)
                .map(|neighbour| (vertex, neighbour)),
        );
    }

    let order = nested_dissection_order(
        &active,
        &edges,
        &NestedDissectionParams {
            salt,
            base_case_size: DEFAULT_BASE_CASE_SIZE,
            max_imbalance: DEFAULT_MAX_IMBALANCE,
            hard_deadline: stop.hard_deadline,
            base_seed: seed,
        },
        0,
    );

    let mut pacer = DeadlinePacer::new();
    for vertex in order {
        if !graph.active[vertex as usize] {
            continue;
        }
        if pacer.due() && expired(stop.hard_deadline) {
            return ElimExit::DeadlineReached(Cutoff::Hard);
        }
        neighbours.clear();
        graph.collect_live_nbrs_into(vertex, &mut neighbours);
        let mut bag = Vec::with_capacity(neighbours.len() + 1);
        bag.push(vertex);
        bag.extend_from_slice(&neighbours);
        let bag_len = bag.len();
        graph.eliminate_with_nbrs(vertex, &neighbours);
        sink.record(vertex, bag);
        if exceeds_width_bound(bag_len, stop.width_bound) {
            return ElimExit::WidthLimitExceeded;
        }
    }
    ElimExit::Complete
}

/// Compute a nested-dissection elimination order for the active vertex set
/// `active` (global IDs) whose internal edges are `edges` (global IDs).
///
/// `depth` counts recursion levels; the top-level caller passes 0. At
/// [`MAX_RECURSION_DEPTH`] the split stops and the subgraph goes to min-fill.
pub(super) fn nested_dissection_order(
    active: &[u32],
    edges: &[(u32, u32)],
    params: &NestedDissectionParams<'_>,
    depth: u32,
) -> Vec<u32> {
    let salt = params.salt;
    let n = active.len();
    if n == 0 {
        return Vec::new();
    }
    if expired(params.hard_deadline) {
        // Return a complete fallback permutation.
        let mut salt_sorted: Vec<u32> = active.to_vec();
        salt_sorted.sort_by_key(|&v| salt[v as usize]);
        return salt_sorted;
    }
    if n <= params.base_case_size || depth >= MAX_RECURSION_DEPTH {
        return base_min_fill_order(active, edges, salt);
    }

    // Relabel active into dense 0..n so multilevel bisection and separator
    // extraction can use vec-indexed adjacency without sparse maps.
    let local_edges = local_edges_for(active, edges);

    let partition_graph = crate::Graph::new(n as u32, local_edges.iter().copied());
    let bisection = multilevel_graph_bisect(
        &partition_graph,
        GraphBisectionConfig::new(params.max_imbalance, params.base_seed),
    )
    .expect("nested-dissection parameters satisfy the bisection contract");
    let sep =
        vertex_cover_separator::minimum_vertex_cover_separator(n, &local_edges, bisection.parts());

    // Degenerate partition — nothing to recurse on. Fall back to local min-fill.
    if sep.side_a.is_empty() || sep.side_b.is_empty() || sep.separator.len() >= n {
        return base_min_fill_order(active, edges, salt);
    }

    let side_a_global = local_to_global(&sep.side_a, active);
    let side_b_global = local_to_global(&sep.side_b, active);
    let sep_global = local_to_global(&sep.separator, active);

    let edges_a = edges_induced_on(edges, &side_a_global);
    let edges_b = edges_induced_on(edges, &side_b_global);

    let mut order = nested_dissection_order(&side_a_global, &edges_a, params, depth + 1);
    order.extend(nested_dissection_order(
        &side_b_global,
        &edges_b,
        params,
        depth + 1,
    ));

    let mut sep_sorted = sep_global;
    sep_sorted.sort_by_key(|&v| salt[v as usize]);
    order.extend(sep_sorted);
    order
}

/// Min-fill on the induced subgraph of `active`, returning the resulting order
/// translated back to global IDs.
fn base_min_fill_order(active: &[u32], edges: &[(u32, u32)], salt: &[u32]) -> Vec<u32> {
    let n = active.len();
    let local_edges = local_edges_for(active, edges);
    let mut local_graph = EliminationGraph::from_edges(n as u32, &local_edges);
    let local_salt: Vec<u32> = active.iter().map(|&v| salt[v as usize]).collect();
    let mut steps = ElimSteps::default();
    eliminate_min_fill(
        &mut local_graph,
        &local_salt,
        steps.sink(),
        ElimStop::default(),
    );
    steps
        .rank_pairs
        .into_iter()
        .map(|(l, _)| active[l as usize])
        .collect()
}

/// Translate `edges` (global IDs) into dense 0..n local IDs where position `i`
/// in `active` becomes local ID `i`. Every endpoint must be in `active`.
///
/// Deliberately NOT
/// [`induced_edges`](crate::graph::induced_edges), which
/// renumbers the same way but hands back a sorted edge list. The order and
/// orientation of what comes out here reach [`EliminationGraph::from_edges`], which fills
/// each adjacency list in the order it is handed, and the base case's min-fill
/// emits each bag in adjacency order — so sorting this list would change the
/// elimination orders this function exists to produce.
fn local_edges_for(active: &[u32], edges: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let to_local = index_by_vertex(active);
    edges
        .iter()
        .map(|&(u, v)| (to_local[&u], to_local[&v]))
        .collect()
}

/// Translate a list of local indices (positions into `active`) back to their
/// original global IDs.
fn local_to_global(locals: &[u32], active: &[u32]) -> Vec<u32> {
    locals.iter().map(|&l| active[l as usize]).collect()
}

fn edges_induced_on(edges: &[(u32, u32)], vertex_set: &[u32]) -> Vec<(u32, u32)> {
    let set: FxHashSet<u32> = vertex_set.iter().copied().collect();
    edges
        .iter()
        .copied()
        .filter(|&(u, v)| set.contains(&u) && set.contains(&v))
        .collect()
}
