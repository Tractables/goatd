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

use std::time::{Duration, Instant};

use super::execution::{
    ElimExit, ElimSink, ElimSteps, ElimStop, eliminate_in_order, residual_edges,
};
use super::graph::EliminationGraph;
use super::greedy::eliminate_min_fill;
use super::vertex_cover_separator;
use crate::deadline::expired;
use crate::partition::{GraphBisectionConfig, multilevel_graph_bisect_until};

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
    /// Hard cutoff, checked at each recursion level and inside the bisection
    /// and the min-fill a level runs. Once reached, the current vertices are
    /// returned in salt order as a complete fallback order.
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
    let (active, adjacency) = residual_edges(graph);
    // The active vertices are in index order, so a neighbour later in that list
    // is also the larger of the two original ids and each edge is emitted once.
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for (index, neighbours) in adjacency.iter().enumerate() {
        for &neighbour in neighbours {
            if neighbour as usize > index {
                edges.push((active[index], active[neighbour as usize]));
            }
        }
    }

    let mut local = LocalIds::new(graph.len());
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
        &mut local,
        0,
    );

    // The order covers the residual whether or not the recursion finished: the
    // levels that got their bisection keep it and the rest are in salt order.
    // Building the bags from it is one pass over the residual, so a run the
    // cutoff stopped is left that much time to finish rather than dropping
    // everything the recursion did.
    let stop = ElimStop {
        hard_deadline: stop.hard_deadline.map(|deadline| {
            deadline
                .checked_add(elimination_allowance(active.len(), edges.len()))
                .unwrap_or(deadline)
        }),
        ..stop
    };
    eliminate_in_order(graph, order, &mut sink, stop)
}

/// What the closing elimination may spend past the recursion's cutoff.
///
/// The pass reads every vertex and every edge of the residual once, and cost
/// 14 ms over 39,985 vertices and 154,053 edges when it was measured, so the
/// projection counts a vertex for four edges and allows a millisecond per ten
/// thousand of them — twice what that measurement needed. A residual whose
/// pass runs longer than its projection is stopped as before and the stage
/// returns nothing. The sum over a graph's components is the projection for
/// the whole graph, so a run cannot buy time by being split into more of them.
fn elimination_allowance(vertices: usize, edges: usize) -> Duration {
    let units = vertices.saturating_mul(4).saturating_add(edges);
    Duration::from_micros((units / 10) as u64)
}

/// Compute a nested-dissection elimination order for the active vertex set
/// `active` (global IDs) whose internal edges are `edges` (global IDs).
///
/// `depth` counts recursion levels; the top-level caller passes 0. At
/// [`MAX_RECURSION_DEPTH`] the split stops and the subgraph goes to min-fill.
///
/// `local` is the scratch index every level renumbers through; it comes back
/// with no vertex marked.
pub(super) fn nested_dissection_order(
    active: &[u32],
    edges: &[(u32, u32)],
    params: &NestedDissectionParams<'_>,
    local: &mut LocalIds,
    depth: u32,
) -> Vec<u32> {
    let salt = params.salt;
    let n = active.len();
    if n == 0 {
        return Vec::new();
    }
    if expired(params.hard_deadline) {
        return salt_order(active, salt);
    }
    if n <= params.base_case_size || depth >= MAX_RECURSION_DEPTH {
        return base_min_fill_order(active, edges, params, local);
    }

    // Relabel active into dense 0..n so multilevel bisection and separator
    // extraction can use vec-indexed adjacency without sparse maps.
    let local_edges = local_edges_for(active, edges, local);

    let partition_graph = crate::Graph::new(n as u32, local_edges.iter().copied());
    let bisection = multilevel_graph_bisect_until(
        &partition_graph,
        GraphBisectionConfig::new(params.max_imbalance, params.base_seed),
        params.hard_deadline,
    )
    .expect("nested-dissection parameters satisfy the bisection contract");
    // The bisection of one level is the longest piece of work in the
    // recursion, and on a graph the coarsening declines to shrink it is
    // seconds of it, so it runs against the same cutoff and hands back nothing
    // when that passes. Same answer as a level that finds the cutoff already
    // reached on the way in.
    let Some(bisection) = bisection else {
        return salt_order(active, salt);
    };
    let sep =
        vertex_cover_separator::minimum_vertex_cover_separator(n, &local_edges, bisection.parts());

    // Degenerate partition — nothing to recurse on. Fall back to local min-fill.
    if sep.side_a.is_empty() || sep.side_b.is_empty() || sep.separator.len() >= n {
        return base_min_fill_order(active, edges, params, local);
    }

    let side_a_global = local_to_global(&sep.side_a, active);
    let side_b_global = local_to_global(&sep.side_b, active);
    let sep_global = local_to_global(&sep.separator, active);

    let edges_a = edges_induced_on(edges, &side_a_global, local);
    let edges_b = edges_induced_on(edges, &side_b_global, local);

    let mut order = nested_dissection_order(&side_a_global, &edges_a, params, local, depth + 1);
    order.extend(nested_dissection_order(
        &side_b_global,
        &edges_b,
        params,
        local,
        depth + 1,
    ));

    let mut sep_sorted = sep_global;
    sep_sorted.sort_by_key(|&v| salt[v as usize]);
    order.extend(sep_sorted);
    order
}

/// The fallback order for a stretch of vertices the recursion has no time to
/// split: a complete permutation of them in salt order.
fn salt_order(active: &[u32], salt: &[u32]) -> Vec<u32> {
    let mut salt_sorted: Vec<u32> = active.to_vec();
    salt_sorted.sort_by_key(|&v| salt[v as usize]);
    salt_sorted
}

/// Min-fill on the induced subgraph of `active`, returning the resulting order
/// translated back to global IDs.
///
/// The min-fill runs against the hard deadline. A degenerate bisection sends a
/// whole level here, and on a dense subgraph of a few thousand vertices an
/// unbounded min-fill takes tens of seconds; when the deadline stops it, the
/// vertices it did not reach follow in salt order, the same fallback a level
/// that finds the deadline already passed returns.
fn base_min_fill_order(
    active: &[u32],
    edges: &[(u32, u32)],
    params: &NestedDissectionParams<'_>,
    local: &mut LocalIds,
) -> Vec<u32> {
    let salt = params.salt;
    let n = active.len();
    let local_edges = local_edges_for(active, edges, local);
    let mut local_graph = EliminationGraph::from_edges(n as u32, &local_edges);
    let local_salt: Vec<u32> = active.iter().map(|&v| salt[v as usize]).collect();
    let mut steps = ElimSteps::default();
    let exit = eliminate_min_fill::<false>(
        &mut local_graph,
        &local_salt,
        steps.sink(),
        ElimStop {
            hard_deadline: params.hard_deadline,
            ..ElimStop::default()
        },
    );
    let mut order: Vec<u32> = steps
        .rank_pairs
        .into_iter()
        .map(|(l, _)| active[l as usize])
        .collect();
    if exit != ElimExit::Complete {
        let mut rest: Vec<u32> = (0..n)
            .filter(|&local| local_graph.active[local])
            .map(|local| active[local])
            .collect();
        rest.sort_by_key(|&v| salt[v as usize]);
        order.extend(rest);
    }
    order
}

/// The entry for a vertex that is not in the set currently marked.
const NOT_IN_SET: u32 = u32::MAX;

/// One array over all of the graph's vertices, holding each vertex's position
/// in the set a level is working on.
///
/// A level renumbers the same edge list three times — once for the bisector and
/// once for each side it recurses on — and the recursion does that at every
/// level, so each lookup is one array read rather than a hash. Marking a set
/// and clearing it again are both linear in the set, so the array itself is
/// never scanned.
pub(super) struct LocalIds {
    position: Vec<u32>,
}

impl LocalIds {
    /// Room for the vertex ids `0..vertices`, nothing marked.
    pub(super) fn new(vertices: usize) -> Self {
        Self {
            position: vec![NOT_IN_SET; vertices],
        }
    }

    /// Run `body` with `set[i]` marked at `i` and every other vertex left at
    /// [`NOT_IN_SET`], then clear the marks again. `set` holds each vertex once.
    fn marking<T>(&mut self, set: &[u32], body: impl FnOnce(&[u32]) -> T) -> T {
        for (position, &vertex) in set.iter().enumerate() {
            self.position[vertex as usize] = position as u32;
        }
        let result = body(&self.position);
        for &vertex in set {
            self.position[vertex as usize] = NOT_IN_SET;
        }
        result
    }
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
fn local_edges_for(active: &[u32], edges: &[(u32, u32)], local: &mut LocalIds) -> Vec<(u32, u32)> {
    local.marking(active, |position| {
        edges
            .iter()
            .map(|&(u, v)| {
                let renumbered = (position[u as usize], position[v as usize]);
                debug_assert!(
                    renumbered.0 != NOT_IN_SET && renumbered.1 != NOT_IN_SET,
                    "edge ({u}, {v}) has an endpoint outside the vertex set being renumbered"
                );
                renumbered
            })
            .collect()
    })
}

/// Translate a list of local indices (positions into `active`) back to their
/// original global IDs.
fn local_to_global(locals: &[u32], active: &[u32]) -> Vec<u32> {
    locals.iter().map(|&l| active[l as usize]).collect()
}

/// The edges of `edges` with both endpoints in `vertex_set`, in global IDs and
/// in the order they were given.
fn edges_induced_on(
    edges: &[(u32, u32)],
    vertex_set: &[u32],
    local: &mut LocalIds,
) -> Vec<(u32, u32)> {
    local.marking(vertex_set, |position| {
        edges
            .iter()
            .copied()
            .filter(|&(u, v)| {
                position[u as usize] != NOT_IN_SET && position[v as usize] != NOT_IN_SET
            })
            .collect()
    })
}
