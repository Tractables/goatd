//! Flow-based hypergraph refinement (a simplified form of Heuer et al., JEA 2018): a
//! max-flow between the two sides of a corridor around the current cut proposes
//! a coordinated relocation of its boundary vertices, complementing the local
//! move-based FM pass. Moves that would violate the requested balance are left
//! out of that proposal before its actual hyperedge cut is evaluated.
//!
//! This phase has no counterpart on the graph side, and runs only at the finest
//! level, from [`refine_finest_level`] below.

use super::initial::hyperedge_cut;
use super::model::Hypergraph;
use super::refine_fm::{localized_fm_pass, refine_level};
use crate::partition::common::balance_bounds;

/// A directed flow network with residual arcs stored in adjacent pairs.
/// `add_edge` creates both arcs together, so `edge ^ 1` is always the reverse
/// arc during augmentation.
pub(super) struct FlowNetwork {
    adjacency: Vec<Vec<(usize, usize)>>,
    residual_capacity: Vec<i64>,
}

impl FlowNetwork {
    pub(super) fn new(num_nodes: usize) -> Self {
        Self {
            adjacency: vec![Vec::new(); num_nodes],
            residual_capacity: Vec::new(),
        }
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
    pub(super) fn max_flow(&mut self, source: usize, sink: usize, source_side: &mut [bool]) -> i64 {
        let mut total_flow = 0i64;
        let mut parent = vec![None; self.adjacency.len()];

        loop {
            parent.fill(None);
            parent[source] = Some((source, 0));
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(source);

            while let Some(node) = queue.pop_front() {
                if node == sink {
                    break;
                }
                for &(neighbor, edge) in &self.adjacency[node] {
                    if parent[neighbor].is_none() && self.residual_capacity[edge] > 0 {
                        parent[neighbor] = Some((node, edge));
                        queue.push_back(neighbor);
                    }
                }
            }

            if parent[sink].is_none() {
                break;
            }

            let mut path_capacity = i64::MAX;
            let mut node = sink;
            while node != source {
                let (previous, edge) = parent[node].expect("a reached node has a parent");
                path_capacity = path_capacity.min(self.residual_capacity[edge]);
                node = previous;
            }

            node = sink;
            while node != source {
                let (previous, edge) = parent[node].expect("an augmenting path has parents");
                self.residual_capacity[edge] -= path_capacity;
                self.residual_capacity[edge ^ 1] += path_capacity;
                node = previous;
            }

            total_flow += path_capacity;
        }

        source_side.fill(false);
        source_side[source] = true;
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(source);
        while let Some(node) = queue.pop_front() {
            for &(neighbor, edge) in &self.adjacency[node] {
                if !source_side[neighbor] && self.residual_capacity[edge] > 0 {
                    source_side[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }

        total_flow
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
pub(super) fn flow_refine(hg: &Hypergraph, part: &mut [u8], max_imbalance: f64) -> bool {
    let n = hg.num_vertices;
    if n < 10 {
        return false;
    }

    let (min_part_weight, max_part_weight) = balance_bounds(&hg.vertex_weights, max_imbalance);

    let pin_counts = hg.pin_counts(part);

    // Corridor: every pin of every cut hyperedge. Interior vertices are left
    // out of the network entirely, which is what keeps it small enough for a
    // whole-corridor max-flow to be worth running.
    let mut is_boundary = vec![false; n];
    let mut cut_hyperedges = Vec::new();
    for (hyperedge, counts) in pin_counts.iter().enumerate() {
        if counts[0] > 0 && counts[1] > 0 {
            cut_hyperedges.push(hyperedge);
            for &vertex in hg.charged_hyperedge_pins(hyperedge) {
                is_boundary[vertex as usize] = true;
            }
        }
    }

    if cut_hyperedges.is_empty() {
        return false;
    }

    let boundary_count = is_boundary.iter().filter(|&&b| b).count();
    if boundary_count > 500 {
        // Tuned cap: max-flow cost grows with corridor size, so large
        // boundary regions skip flow refinement rather than pay for it.
        return false;
    }

    // Flow-network node-ID layout: source (0), sink (1), boundary vertices
    // (2..2+boundary_count), cut hyperedge nodes (2+boundary_count..).
    let mut node_by_vertex = vec![None; n];
    let mut boundary_vertices = Vec::new();
    let mut next_node = 2usize;
    for v in 0..n {
        if is_boundary[v] {
            node_by_vertex[v] = Some(next_node);
            boundary_vertices.push(v);
            next_node += 1;
        }
    }
    let hyperedge_node_start = next_node;
    let total_nodes = hyperedge_node_start + cut_hyperedges.len();
    let source = 0;
    let sink = 1;

    let mut network = FlowNetwork::new(total_nodes);

    // Source feeds partition-0 boundary vertices; partition-1 ones feed sink.
    // A vertex therefore ends on side 0 exactly when the residual graph still
    // reaches it from the source, and cutting its terminal edge is what the
    // network charges for relocating it.
    for &vertex in &boundary_vertices {
        let vertex_node = node_by_vertex[vertex].expect("a boundary vertex has a flow node");
        if part[vertex] == 0 {
            network.add_edge(source, vertex_node, i64::from(hg.vertex_weights[vertex]));
        } else {
            network.add_edge(vertex_node, sink, i64::from(hg.vertex_weights[vertex]));
        }
    }

    // Boundary-vertex-to-cut-hyperedge capacity equals the hyperedge weight,
    // so the min-cut cost matches the hg cut it approximates.
    for (cut_index, &hyperedge) in cut_hyperedges.iter().enumerate() {
        let hyperedge_node = hyperedge_node_start + cut_index;
        let hyperedge_weight = i64::from(hg.hyperedge_weights[hyperedge]);
        for &vertex in hg.charged_hyperedge_pins(hyperedge) {
            let vertex = vertex as usize;
            let vertex_node =
                node_by_vertex[vertex].expect("every cut-hyperedge pin has a flow node");
            if part[vertex] == 0 {
                network.add_edge(vertex_node, hyperedge_node, hyperedge_weight);
            } else {
                network.add_edge(hyperedge_node, vertex_node, hyperedge_weight);
            }
        }
    }

    let mut reachable_from_source = vec![false; total_nodes];
    network.max_flow(source, sink, &mut reachable_from_source);

    let mut proposal = part.to_vec();
    let mut part_weight = [0u32; 2];
    for vertex in 0..n {
        part_weight[part[vertex] as usize] += hg.vertex_weights[vertex];
    }

    let mut changed = false;
    for &vertex in &boundary_vertices {
        let vertex_node = node_by_vertex[vertex].expect("a boundary vertex has a flow node");
        let new_side = u8::from(!reachable_from_source[vertex_node]);
        if new_side != part[vertex] {
            let from = part[vertex] as usize;
            let to = new_side as usize;
            let weight_after_leaving = part_weight[from] - hg.vertex_weights[vertex];
            let weight_after_joining = part_weight[to] + hg.vertex_weights[vertex];
            if weight_after_leaving >= min_part_weight && weight_after_joining <= max_part_weight {
                proposal[vertex] = new_side;
                part_weight[from] = weight_after_leaving;
                part_weight[to] = weight_after_joining;
                changed = true;
            }
        }
    }

    if changed {
        let old_cut = hyperedge_cut(hg, part);
        let new_cut = hyperedge_cut(hg, &proposal);
        if new_cut < old_cut {
            part.copy_from_slice(&proposal);
            return true;
        }
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
pub(super) fn refine_finest_level(hg: &Hypergraph, part: &mut [u8], imbalance: f64) {
    refine_level(hg, part, imbalance);

    let n = hg.num_vertices;
    if n < 20 {
        return;
    }

    // 7919 below is prime, so successive tries land in unrelated stretches of
    // the boundary list rather than in one region's worth of adjacent vertices.
    let num_tries = 4.min(n);
    let mut boundary: Vec<usize> = Vec::new();
    {
        let pin_counts = hg.pin_counts(part);
        for v in 0..n {
            for &hei in hg.vertex_hyperedges(v) {
                if pin_counts[hei as usize][0] > 0 && pin_counts[hei as usize][1] > 0 {
                    boundary.push(v);
                    break;
                }
            }
        }
    }
    if !boundary.is_empty() {
        for i in 0..num_tries {
            let seed = boundary[(i * 7919) % boundary.len()];
            localized_fm_pass(hg, part, seed, imbalance);
        }
    }

    flow_refine(hg, part, imbalance);
}
