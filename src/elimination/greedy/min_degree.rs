//! Min-degree elimination: repeatedly remove the active vertex of lowest
//! current degree. The ordinary form breaks ties by the caller's salt and
//! then by vertex id; the update-order form prefers the oldest heap entry.
//!
//! One instantiation of the greedy skeleton in `greedy`. The elimination engine runs it as
//! a candidate of the elimination portfolio; the bags it emits through the sink are
//! what a tree decomposition is built from.

use super::deterministic::{AfterElim, ElimPolicy, eliminate_greedy};
use super::*;

/// Heap entry for min-degree — (degree, tie key, vertex) ascending.
#[derive(Eq, PartialEq)]
pub(super) struct DegEntry {
    /// Ordering key: `(degree, tie key, vertex)`.
    pub key: (Reverse<u64>, Reverse<u64>, Reverse<u32>),
    pub vertex: u32,
    /// Duplicated out of `key` so the stale-snapshot guard
    /// (`graph.degree(v) != degree`) can read it without destructuring the
    /// `Reverse`-wrapped tuple.
    pub degree: u64,
}

impl DegEntry {
    pub(super) fn new(degree: u64, tie: u64, v: u32) -> Self {
        DegEntry {
            key: (Reverse(degree), Reverse(tie), Reverse(v)),
            vertex: v,
            degree,
        }
    }
}

ord_by_key!(DegEntry);

impl ElimEntry for DegEntry {
    fn vertex(&self) -> u32 {
        self.vertex
    }
    fn snapshot(&self) -> u64 {
        self.degree
    }
}

/// Greedy min-degree: rank by current degree, break ties by salt. This is the
/// skeleton's plainest instance — it takes every default, including owing its
/// neighbours nothing when a vertex is eliminated. The stale-snapshot guard
/// corrects an out-of-date entry when it surfaces, which keeps the heap at
/// O(n) entries instead of accumulating one per degree change.
struct MinDegree<'a> {
    heap: BinaryHeap<DegEntry>,
    salt: &'a [u32],
    update_order_ties: bool,
    next_update: u64,
}

impl ElimPolicy for MinDegree<'_> {
    type Entry = DegEntry;

    const CHEAP_MODE: bool = true;
    // Degree is a single lookup either way, so the bitset would buy nothing.
    const MAINTAIN_BITSET: bool = false;
    // Ranking by degree says nothing about whether N(v) is already a clique.
    const ZERO_SCORE_IS_SIMPLICIAL: bool = false;

    fn heap(&mut self) -> &mut BinaryHeap<DegEntry> {
        &mut self.heap
    }

    fn push(&mut self, _: &EliminationGraph, v: u32, score: u64) {
        let tie = if self.update_order_ties {
            let update = self.next_update;
            self.next_update += 1;
            update
        } else {
            self.salt[v as usize] as u64
        };
        self.heap.push(DegEntry::new(score, tie, v));
    }

    fn live_score(&mut self, graph: &EliminationGraph, v: u32) -> u64 {
        graph.degree(v) as u64
    }

    fn after_eliminate(
        &mut self,
        graph: &EliminationGraph,
        nbrs: &[u32],
        cheap_mode: bool,
        _: Option<Instant>,
        _: bool,
    ) -> AfterElim {
        if self.update_order_ties && !cheap_mode {
            for &vertex in nbrs {
                if graph.active[vertex as usize] {
                    self.push(graph, vertex, graph.degree(vertex) as u64);
                }
            }
        }
        AfterElim::Continue
    }
}

/// Pure min-degree elimination. The ordinary form ranks by
/// `(degree, salt, vertex)`; the update-order form replaces the salt with a
/// monotonically increasing key whenever a changed neighbour is reinserted.
/// Both are cheaper than min-fill because they skip fill recomputation.
pub(crate) fn eliminate_min_degree(
    graph: &mut EliminationGraph,
    salt: &[u32],
    update_order_ties: bool,
    sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    let n = graph.len();
    assert_eq!(salt.len(), n);
    let mut policy = MinDegree {
        heap: BinaryHeap::with_capacity(n),
        salt,
        update_order_ties,
        next_update: 0,
    };
    eliminate_greedy(&mut policy, graph, sink, stop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elimination::execution::ElimSteps;

    #[test]
    fn update_order_ties_prefer_a_recently_exposed_leaf() {
        let mut graph = EliminationGraph::from_edges(5, &[(0, 1), (0, 2), (0, 4), (1, 2), (3, 4)]);
        let salt = vec![0; 5];
        let mut steps = ElimSteps::default();

        let exit = eliminate_min_degree(&mut graph, &salt, true, steps.sink(), ElimStop::default());

        assert_eq!(exit, ElimExit::Complete);
        let order: Vec<u32> = steps
            .rank_pairs
            .into_iter()
            .map(|(vertex, _)| vertex)
            .collect();
        assert_eq!(order, [3, 4, 1, 0, 2]);
    }
}
