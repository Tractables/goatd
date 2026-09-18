//! Flow-based hypergraph refinement (a simplified form of Heuer et al., JEA 2018): a
//! max-flow between the two sides of a corridor around the current cut proposes
//! a coordinated relocation of its boundary vertices, complementing the local
//! move-based FM pass. Moves that would violate the requested balance are left
//! out of that proposal before its actual hyperedge cut is evaluated.
//!
//! This phase has no counterpart on the graph side, and runs only at the finest
//! level, from [`refine_finest_level`] below.

use std::collections::VecDeque;

use super::initial::hyperedge_cut;
use super::model::Hypergraph;
use super::refine_fm::{FmScratch, localized_fm_pass, refine_level};
use crate::partition::common::{BisectionStop, balance_bounds};

/// A directed flow network with residual arcs stored in adjacent pairs.
/// `add_edge` creates both arcs together, so `edge ^ 1` is always the reverse
/// arc during augmentation.
pub(super) struct FlowNetwork {
    adjacency: Vec<Vec<(usize, usize)>>,
    residual_capacity: Vec<i64>,
    parent: Vec<Option<(usize, usize)>>,
    queue: VecDeque<usize>,
}

impl FlowNetwork {
    pub(super) fn new(num_nodes: usize) -> Self {
        let mut network = Self {
            adjacency: Vec::new(),
            residual_capacity: Vec::new(),
            parent: Vec::new(),
            queue: VecDeque::new(),
        };
        network.reset(num_nodes);
        network
    }

    /// Empty the network and size it for `num_nodes`. Each adjacency list is
    /// cleared rather than dropped, so a network built again at the next level
    /// reuses the lists the last one grew.
    pub(super) fn reset(&mut self, num_nodes: usize) {
        for list in &mut self.adjacency {
            list.clear();
        }
        self.adjacency.resize_with(num_nodes, Vec::new);
        self.residual_capacity.clear();
        self.parent.clear();
        self.parent.resize(num_nodes, None);
    }

    pub(super) fn add_edge(&mut self, from: usize, to: usize, capacity: i64) {
        let edge = self.residual_capacity.len();
        self.adjacency[from].push((to, edge));
        self.residual_capacity.push(capacity);
        self.adjacency[to].push((from, edge + 1));
        self.residual_capacity.push(0);
    }

    /// Edmonds-Karp max flow from `source` to `sink`. `source_side` receives
    /// the vertices reachable from `source` in the final residual graph.
    ///
    /// Each augmentation is a breadth-first search of the whole network, so the
    /// clock is read once per augmentation. A stopped search returns the flow
    /// it had reached, and the reachability scan below then gives a cut that is
    /// valid but need not be minimal; the caller adopts a proposal only where
    /// the hyperedge cut actually drops, so a stopped flow cannot make the
    /// partition worse.
    pub(super) fn max_flow(
        &mut self,
        source: usize,
        sink: usize,
        source_side: &mut [bool],
        stop: &mut BisectionStop,
    ) -> i64 {
        let mut total_flow = 0i64;

        loop {
            if stop.reached() {
                break;
            }
            self.parent.fill(None);
            self.parent[source] = Some((source, 0));
            self.queue.clear();
            self.queue.push_back(source);

            while let Some(node) = self.queue.pop_front() {
                if node == sink {
                    break;
                }
                for &(neighbor, edge) in &self.adjacency[node] {
                    if self.parent[neighbor].is_none() && self.residual_capacity[edge] > 0 {
                        self.parent[neighbor] = Some((node, edge));
                        self.queue.push_back(neighbor);
                    }
                }
            }

            if self.parent[sink].is_none() {
                break;
            }

            let mut path_capacity = i64::MAX;
            let mut node = sink;
            while node != source {
                let (previous, edge) = self.parent[node].expect("a reached node has a parent");
                path_capacity = path_capacity.min(self.residual_capacity[edge]);
                node = previous;
            }

            node = sink;
            while node != source {
                let (previous, edge) = self.parent[node].expect("an augmenting path has parents");
                self.residual_capacity[edge] -= path_capacity;
                self.residual_capacity[edge ^ 1] += path_capacity;
                node = previous;
            }

            total_flow += path_capacity;
        }

        source_side.fill(false);
        source_side[source] = true;
        self.queue.clear();
        self.queue.push_back(source);
        while let Some(node) = self.queue.pop_front() {
            for &(neighbor, edge) in &self.adjacency[node] {
                if !source_side[neighbor] && self.residual_capacity[edge] > 0 {
                    source_side[neighbor] = true;
                    self.queue.push_back(neighbor);
                }
            }
        }

        total_flow
    }
}

/// The largest corridor the flow pass will build a network over.
const MAX_CORRIDOR: usize = 500;

/// The largest network it will build over one. The corridor cap bounds the
/// vertex side — a node and a terminal arc each — and this bounds the other:
/// the cut hyperedges, a node each and an arc per pin. Nothing else bounds
/// them, and they are not bounded by the corridor, because the same few
/// hundred vertices can be the cut pins of any number of hyperedges. A
/// variable in a great many clauses is that shape.
///
/// Read it as sixty-four cut hyperedges per corridor vertex, which is far
/// above what a corridor of a few hundred vertices ordinarily carries: this is
/// a bound on the structure, like the bitset's size cap, not a gate tuned
/// against a time budget.
const MAX_CORRIDOR_ARCS: usize = 64 * MAX_CORRIDOR;

/// Working storage for the finest level: the boundary the localized passes are
/// seeded from, and everything [`flow_refine`] builds its network out of. Held
/// across the levels of a sweep and across the sweeps of a bisection.
///
/// A corridor is at most [`MAX_CORRIDOR`] vertices out of `n`, so the
/// per-vertex arrays are stamped with the pass number rather than cleared: a
/// vertex is in the corridor when its stamp is this pass's, and a pass that
/// declines leaves nothing behind to undo.
pub(super) struct FinestScratch {
    pub(super) boundary: Vec<usize>,
    stamp: Vec<u32>,
    pass: u32,
    node_by_vertex: Vec<u32>,
    corridor: Vec<usize>,
    cut_hyperedges: Vec<usize>,
    reachable: Vec<bool>,
    moves: Vec<(usize, u8)>,
    network: FlowNetwork,
}

impl FinestScratch {
    pub(super) fn new() -> Self {
        FinestScratch {
            boundary: Vec::new(),
            stamp: Vec::new(),
            pass: 0,
            node_by_vertex: Vec::new(),
            corridor: Vec::new(),
            cut_hyperedges: Vec::new(),
            reachable: Vec::new(),
            moves: Vec::new(),
            network: FlowNetwork::new(0),
        }
    }

    /// Start a flow pass over `n` vertices. The stamp moves on instead of the
    /// arrays being cleared; the one clear a run needs is at the wrap.
    fn prepare(&mut self, n: usize) {
        self.stamp.resize(n, 0);
        self.node_by_vertex.resize(n, u32::MAX);
        self.pass = match self.pass.checked_add(1) {
            Some(pass) => pass,
            None => {
                self.stamp.fill(0);
                1
            }
        };
        self.corridor.clear();
        self.cut_hyperedges.clear();
        self.moves.clear();
    }
}

/// Models cut hyperedges as flow-network nodes; the min cut proposes new sides
/// for the boundary vertices.
///
/// The min cut prices two different things in one number: cutting a
/// vertex-to-terminal edge costs vertex weight and cutting a hyperedge edge
/// costs hyperedge weight. The two are not commensurate, so the flow optimum is not the
/// hyperedge-cut optimum. Balance filtering can also retain only part of the
/// proposal. The resulting bisection is adopted only when `cut` actually
/// drops. Returns whether it did.
pub(super) fn flow_refine(
    hg: &Hypergraph,
    part: &mut [u8],
    max_imbalance: f64,
    scratch: &mut FmScratch,
    stop: &mut BisectionStop,
) -> bool {
    let n = hg.vertex_count;
    if n < 10 {
        return false;
    }

    let (min_part_weight, max_part_weight) = balance_bounds(&hg.vertex_weights, max_imbalance);

    hg.fill_pin_counts(part, &mut scratch.pin_counts);
    let pin_counts = scratch.pin_counts.as_slice();
    let finest = &mut scratch.finest;
    finest.prepare(n);

    // Corridor: every pin of every cut hyperedge. Interior vertices are left
    // out of the network entirely, which is what keeps it small enough for a
    // whole-corridor max-flow to be worth running.
    //
    // Max-flow cost grows with the network, so a corridor over either cap
    // skips the pass rather than pays for it. Both counts are kept as the
    // corridor is marked, so a hypergraph far over a cap stops at it instead
    // of walking every cut hyperedge's pins first.
    let mut arcs = 0usize;
    for (hyperedge, counts) in pin_counts.iter().enumerate() {
        if counts[0] > 0 && counts[1] > 0 {
            finest.cut_hyperedges.push(hyperedge);
            arcs += usize::try_from(counts[0] + counts[1]).unwrap_or(usize::MAX);
            for &vertex in hg.charged_hyperedge_pins(hyperedge) {
                let vertex = vertex as usize;
                if finest.stamp[vertex] != finest.pass {
                    finest.stamp[vertex] = finest.pass;
                    finest.corridor.push(vertex);
                }
            }
            if finest.corridor.len() > MAX_CORRIDOR || arcs > MAX_CORRIDOR_ARCS {
                // The pins the walk stops short of are charged all the same: a
                // budgeted run repeats on the meter, so skipped work still has
                // to pay for itself. A cut hyperedge's two counts add up to its
                // pin count.
                let unwalked: u64 = pin_counts[hyperedge + 1..]
                    .iter()
                    .filter(|rest| rest[0] > 0 && rest[1] > 0)
                    .map(|rest| u64::from(rest[0]) + u64::from(rest[1]))
                    .sum();
                crate::meter::charge(unwalked);
                return false;
            }
        }
    }

    if finest.cut_hyperedges.is_empty() {
        return false;
    }

    // Flow-network node-ID layout: source (0), sink (1), corridor vertices
    // (2..2+corridor), cut hyperedge nodes (2+corridor..). The corridor is
    // sorted rather than collected by a scan of the whole vertex range, which
    // is the same ascending order over at most `MAX_CORRIDOR` entries.
    finest.corridor.sort_unstable();
    let mut next_node = 2usize;
    for &vertex in &finest.corridor {
        finest.node_by_vertex[vertex] = next_node as u32;
        next_node += 1;
    }
    let hyperedge_node_start = next_node;
    let total_nodes = hyperedge_node_start + finest.cut_hyperedges.len();
    let source = 0;
    let sink = 1;

    finest.network.reset(total_nodes);

    // Source feeds partition-0 boundary vertices; partition-1 ones feed sink.
    // A vertex therefore ends on side 0 exactly when the residual graph still
    // reaches it from the source, and cutting its terminal edge is what the
    // network charges for relocating it.
    for &vertex in &finest.corridor {
        let vertex_node = finest.node_by_vertex[vertex] as usize;
        if part[vertex] == 0 {
            finest
                .network
                .add_edge(source, vertex_node, i64::from(hg.vertex_weights[vertex]));
        } else {
            finest
                .network
                .add_edge(vertex_node, sink, i64::from(hg.vertex_weights[vertex]));
        }
    }

    // Boundary-vertex-to-cut-hyperedge capacity equals the hyperedge weight,
    // so the min-cut cost matches the hg cut it approximates.
    for (cut_index, &hyperedge) in finest.cut_hyperedges.iter().enumerate() {
        let hyperedge_node = hyperedge_node_start + cut_index;
        let hyperedge_weight = i64::from(hg.hyperedge_weights[hyperedge]);
        for &vertex in hg.charged_hyperedge_pins(hyperedge) {
            let vertex = vertex as usize;
            debug_assert_eq!(
                finest.stamp[vertex], finest.pass,
                "every cut-hyperedge pin is in the corridor"
            );
            let vertex_node = finest.node_by_vertex[vertex] as usize;
            if part[vertex] == 0 {
                finest
                    .network
                    .add_edge(vertex_node, hyperedge_node, hyperedge_weight);
            } else {
                finest
                    .network
                    .add_edge(hyperedge_node, vertex_node, hyperedge_weight);
            }
        }
    }

    finest.reachable.resize(total_nodes, false);
    finest
        .network
        .max_flow(source, sink, &mut finest.reachable, stop);

    let mut part_weight = [0u32; 2];
    for vertex in 0..n {
        part_weight[part[vertex] as usize] += hg.vertex_weights[vertex];
    }

    for &vertex in &finest.corridor {
        let vertex_node = finest.node_by_vertex[vertex] as usize;
        let new_side = u8::from(!finest.reachable[vertex_node]);
        if new_side != part[vertex] {
            let from = part[vertex] as usize;
            let to = new_side as usize;
            let weight_after_leaving = part_weight[from] - hg.vertex_weights[vertex];
            let weight_after_joining = part_weight[to] + hg.vertex_weights[vertex];
            if weight_after_leaving >= min_part_weight && weight_after_joining <= max_part_weight {
                finest.moves.push((vertex, new_side));
                part_weight[from] = weight_after_leaving;
                part_weight[to] = weight_after_joining;
            }
        }
    }

    if finest.moves.is_empty() {
        return false;
    }

    // The proposal goes onto `part` and comes back off where it does not pay.
    // There are two sides, so a move's other side is the one it came from.
    let old_cut = hyperedge_cut(hg, part);
    for &(vertex, new_side) in &finest.moves {
        part[vertex] = new_side;
    }
    if hyperedge_cut(hg, part) < old_cut {
        return true;
    }
    for &(vertex, new_side) in &finest.moves {
        part[vertex] = 1 - new_side;
    }
    false
}

/// FM refinement with multi-try localized passes and flow-based refinement.
///
/// The finest level's entry point, and the only caller of `flow_refine`:
/// global FM first, then localized passes seeded around the boundary, then one
/// flow pass over what is left. Each stage starts from the previous stage's
/// output, and the flow pass declines outright on a boundary wider than its own
/// cap, so the FM stages also decide whether it runs at all.
pub(super) fn refine_finest_level(
    hg: &Hypergraph,
    part: &mut [u8],
    max_imbalance: f64,
    scratch: &mut FmScratch,
    stop: &mut BisectionStop,
) {
    refine_level(hg, part, max_imbalance, scratch, stop);

    let n = hg.vertex_count;
    if n < 20 || stop.stopped() {
        return;
    }

    // 7919 below is prime, so successive tries land in unrelated stretches of
    // the boundary list rather than in one region's worth of adjacent vertices.
    let num_tries = 4;
    hg.fill_pin_counts(part, &mut scratch.pin_counts);
    {
        let pin_counts = scratch.pin_counts.as_slice();
        let boundary = &mut scratch.finest.boundary;
        boundary.clear();
        for v in 0..n {
            for &hei in hg.vertex_hyperedges(v) {
                if pin_counts[hei as usize][0] > 0 && pin_counts[hei as usize][1] > 0 {
                    boundary.push(v);
                    break;
                }
            }
        }
    }
    if !scratch.finest.boundary.is_empty() {
        for i in 0..num_tries {
            if stop.stopped() {
                return;
            }
            let boundary = &scratch.finest.boundary;
            let seed = boundary[(i * 7919) % boundary.len()];
            localized_fm_pass(hg, part, seed, max_imbalance, &mut scratch.region, stop);
        }
    }

    flow_refine(hg, part, max_imbalance, scratch, stop);
}
