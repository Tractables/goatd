//! Min-degree elimination: repeatedly remove the active vertex of lowest
//! current degree. The ordinary form breaks ties by the caller's salt and
//! then by vertex id; the update-order form replaces the salt with a counter
//! that advances every time a vertex is refiled, so the vertex whose degree
//! changed longest ago goes first.
//!
//! One instantiation of the greedy skeleton in `greedy`. The elimination engine runs it as
//! a candidate of the elimination portfolio; the bags it emits through the sink are
//! what a tree decomposition is built from.

use super::degree_queue::DegreeQueue;
use super::deterministic::{AfterElim, ElimPolicy, eliminate_greedy};
use super::*;

/// Queue entry for min-degree: the vertex and the degree it was filed under.
/// The queue keeps one entry per active vertex and orders them itself, so an
/// entry carries no ordering key of its own.
pub(super) struct DegEntry {
    pub vertex: u32,
    pub degree: u64,
}

impl ElimEntry for DegEntry {
    fn vertex(&self) -> u32 {
        self.vertex
    }
    fn snapshot(&self) -> u64 {
        self.degree
    }
}

/// Greedy min-degree: rank by current degree, break ties by salt. Degree is a
/// small integer and changes only for the neighbours of the vertex just
/// eliminated, so the queue is a bucket per degree and an elimination refiles
/// its neighbours in place.
struct MinDegree<'a> {
    queue: DegreeQueue,
    salt: &'a [u32],
    update_order_ties: bool,
    next_update: u64,
}

impl ElimPolicy for MinDegree<'_> {
    type Entry = DegEntry;

    const CHEAP_MODE: bool = true;
    // Scoring is a single lookup either way, but the elimination is not: on a
    // dense residual it tests every pair of neighbours for an existing edge,
    // and against rows that is the degree squared probes while against a
    // bitset it is the degree times the row's words.
    const MAINTAIN_BITSET: bool = true;
    // Ranking by degree says nothing about whether N(v) is already a clique.
    const ZERO_SCORE_IS_SIMPLICIAL: bool = false;

    fn pop(&mut self) -> Option<DegEntry> {
        self.queue
            .pop_min()
            .map(|(degree, _, vertex)| DegEntry { vertex, degree })
    }

    fn push(&mut self, _: &EliminationGraph, v: u32, score: u64) {
        let tie = if self.update_order_ties {
            let update = self.next_update;
            self.next_update += 1;
            update
        } else {
            self.salt[v as usize] as u64
        };
        self.queue.set(v, score, tie);
    }

    fn live_score(&mut self, graph: &EliminationGraph, v: u32) -> u64 {
        graph.degree(v) as u64
    }

    /// Nothing to re-check. The queue holds one entry per active vertex and
    /// [`after_eliminate`](Self::after_eliminate) refiles every vertex whose
    /// degree an elimination changed, so a popped entry's degree is the live
    /// one. In cheap mode the skeleton skips this call anyway.
    fn rescore_on_pop(&mut self, _: &EliminationGraph, _: u32) -> Option<u64> {
        None
    }

    /// Refile the neighbours whose degree the elimination changed — the only
    /// vertices it can have changed, and only those of them the fill did not
    /// leave at the same degree. Each is one removal and one insert in the
    /// queue, not a second entry left to be discarded later.
    ///
    /// Skipping a neighbour whose degree came out unchanged is what keeps the
    /// update-order key faithful to the heap this replaced: that heap pushed
    /// every neighbour, but a push carrying an unchanged degree only added an
    /// entry the older one still outranked, so the vertex kept its earlier
    /// position in the update order.
    ///
    /// In cheap mode score maintenance is off: the queue keeps whatever
    /// degrees it last recorded and the remaining vertices come out in that
    /// stale order.
    fn after_eliminate(
        &mut self,
        graph: &EliminationGraph,
        nbrs: &[u32],
        cheap_mode: bool,
        _: Option<Instant>,
        _: bool,
    ) -> AfterElim {
        if !cheap_mode {
            for &vertex in nbrs {
                if graph.active[vertex as usize] {
                    let degree = graph.degree(vertex) as u64;
                    if self.queue.degree_of(vertex) != Some(degree) {
                        self.push(graph, vertex, degree);
                    }
                }
            }
        }
        AfterElim::Continue
    }
}

/// Pure min-degree elimination. The ordinary form ranks by
/// `(degree, salt, vertex)`; the update-order form replaces the salt with a
/// monotonically increasing key whenever a changed neighbour is refiled.
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
        queue: DegreeQueue::new(n),
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
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    /// The heap min-degree the bucket queue replaced: every score change is a
    /// second entry, and the older one is discarded when it surfaces. Kept
    /// here as the reference the bucket queue has to reproduce.
    #[derive(Eq, PartialEq)]
    struct RefEntry {
        key: (Reverse<u64>, Reverse<u64>, Reverse<u32>),
        vertex: u32,
        degree: u64,
    }

    ord_by_key!(RefEntry);

    impl ElimEntry for RefEntry {
        fn vertex(&self) -> u32 {
            self.vertex
        }
        fn snapshot(&self) -> u64 {
            self.degree
        }
    }

    struct RefMinDegree<'a> {
        heap: BinaryHeap<RefEntry>,
        salt: &'a [u32],
        update_order_ties: bool,
        next_update: u64,
    }

    impl ElimPolicy for RefMinDegree<'_> {
        type Entry = RefEntry;

        const CHEAP_MODE: bool = true;
        const MAINTAIN_BITSET: bool = true;
        const ZERO_SCORE_IS_SIMPLICIAL: bool = false;

        fn pop(&mut self) -> Option<RefEntry> {
            self.heap.pop()
        }

        fn push(&mut self, _: &EliminationGraph, v: u32, score: u64) {
            let tie = if self.update_order_ties {
                let update = self.next_update;
                self.next_update += 1;
                update
            } else {
                self.salt[v as usize] as u64
            };
            self.heap.push(RefEntry {
                key: (Reverse(score), Reverse(tie), Reverse(v)),
                vertex: v,
                degree: score,
            });
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
            if !cheap_mode {
                for &vertex in nbrs {
                    if graph.active[vertex as usize] {
                        self.push(graph, vertex, graph.degree(vertex) as u64);
                    }
                }
            }
            AfterElim::Continue
        }
    }

    fn reference_steps(
        graph: &mut EliminationGraph,
        salt: &[u32],
        update_order_ties: bool,
    ) -> ElimSteps {
        let mut policy = RefMinDegree {
            heap: BinaryHeap::with_capacity(graph.len()),
            salt,
            update_order_ties,
            next_update: 0,
        };
        let mut steps = ElimSteps::default();
        let exit = eliminate_greedy(&mut policy, graph, steps.sink(), ElimStop::default());
        assert_eq!(exit, ElimExit::Complete);
        steps
    }

    fn bucket_steps(
        graph: &mut EliminationGraph,
        salt: &[u32],
        update_order_ties: bool,
    ) -> ElimSteps {
        let mut steps = ElimSteps::default();
        let exit = eliminate_min_degree(
            graph,
            salt,
            update_order_ties,
            steps.sink(),
            ElimStop::default(),
        );
        assert_eq!(exit, ElimExit::Complete);
        steps
    }

    fn cycle(n: u32) -> Vec<(u32, u32)> {
        (0..n).map(|v| (v, (v + 1) % n)).collect()
    }

    fn grid(side: u32) -> Vec<(u32, u32)> {
        let mut edges = Vec::new();
        for r in 0..side {
            for c in 0..side {
                let v = r * side + c;
                if c + 1 < side {
                    edges.push((v, v + 1));
                }
                if r + 1 < side {
                    edges.push((v, v + side));
                }
            }
        }
        edges
    }

    /// A deterministic pseudo-random graph, so the check runs on something
    /// with an uneven degree distribution as well as the regular graphs.
    fn scattered(n: u32, edges_wanted: usize) -> Vec<(u32, u32)> {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut edges = Vec::with_capacity(edges_wanted);
        while edges.len() < edges_wanted {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let u = (state % u64::from(n)) as u32;
            let v = ((state >> 32) % u64::from(n)) as u32;
            if u != v {
                edges.push((u, v));
            }
        }
        edges
    }

    /// A named graph to check both queues on.
    struct Case {
        name: &'static str,
        vertices: u32,
        edges: Vec<(u32, u32)>,
    }

    fn case(name: &'static str, vertices: u32, edges: Vec<(u32, u32)>) -> Case {
        Case {
            name,
            vertices,
            edges,
        }
    }

    fn cases() -> Vec<Case> {
        vec![
            case("five", 5, vec![(0, 1), (0, 2), (0, 4), (1, 2), (3, 4)]),
            case("cycle12", 12, cycle(12)),
            case("grid4", 16, grid(4)),
            case("scattered40", 40, scattered(40, 140)),
            case("scattered60dense", 60, scattered(60, 600)),
        ]
    }

    fn salts(n: u32, varied: bool) -> Vec<u32> {
        if varied {
            (0..n)
                .map(|v| v.wrapping_mul(2_654_435_761) % 1_000)
                .collect()
        } else {
            vec![0; n as usize]
        }
    }

    /// Every degree in a cycle is 2 and every salt is 0, so the whole first
    /// pass is one tie: if the bucket queue ordered a tie differently from the
    /// heap it would show here.
    #[test]
    fn the_bucket_queue_eliminates_in_the_heap_order() {
        for Case {
            name,
            vertices,
            edges,
        } in cases()
        {
            for varied in [false, true] {
                let salt = salts(vertices, varied);
                let mut a = EliminationGraph::from_edges(vertices, &edges);
                let mut b = EliminationGraph::from_edges(vertices, &edges);
                let reference = reference_steps(&mut a, &salt, false);
                let bucket = bucket_steps(&mut b, &salt, false);
                assert_eq!(
                    reference.rank_pairs, bucket.rank_pairs,
                    "{name} (varied salt: {varied}) eliminated in a different order"
                );
                assert_eq!(
                    reference.bags, bucket.bags,
                    "{name} (varied salt: {varied}) emitted different bags"
                );
            }
        }
    }

    /// A cycle is all one degree, so the update-order key decides every step
    /// of the first pass. The heap this replaced pushed a neighbour even when
    /// the fill left its degree alone, and the older entry still outranked
    /// that push; the queue reproduces it by refiling only on a real change.
    #[test]
    fn the_update_order_form_keeps_the_heap_order_on_equal_degrees() {
        let edges = cycle(12);
        let salt = salts(12, true);
        let mut a = EliminationGraph::from_edges(12, &edges);
        let mut b = EliminationGraph::from_edges(12, &edges);
        let reference = reference_steps(&mut a, &salt, true);
        let bucket = bucket_steps(&mut b, &salt, true);
        assert_eq!(reference.rank_pairs, bucket.rank_pairs);
        assert_eq!(reference.bags, bucket.bags);
    }

    /// Where the update-order form does reorder, it is not free to reorder
    /// anything: the bag sizes are the same multiset, so the width and the bag
    /// mass do not move.
    ///
    /// The reorder itself is a property of the heap, not of the queue. A heap
    /// keeps every superseded entry, so a vertex whose degree returns to a
    /// value it held before is ranked by the counter of the older entry that
    /// still carries that value. A queue holding one entry per vertex has only
    /// the counter of the most recent change. `grid4` hits this and swaps the
    /// last four eliminations; the salt form, whose tie key does not move, is
    /// identical on every graph here.
    #[test]
    fn the_update_order_form_keeps_the_bag_sizes_where_it_reorders() {
        for Case {
            name,
            vertices,
            edges,
        } in cases()
        {
            let salt = salts(vertices, true);
            let mut a = EliminationGraph::from_edges(vertices, &edges);
            let mut b = EliminationGraph::from_edges(vertices, &edges);
            let reference = reference_steps(&mut a, &salt, true);
            let bucket = bucket_steps(&mut b, &salt, true);
            let mut reference_sizes: Vec<usize> = reference.bags.iter().map(Vec::len).collect();
            let mut bucket_sizes: Vec<usize> = bucket.bags.iter().map(Vec::len).collect();
            reference_sizes.sort_unstable();
            bucket_sizes.sort_unstable();
            assert_eq!(
                reference_sizes, bucket_sizes,
                "{name} changed its bag sizes"
            );
        }
    }

    fn elimination_order(update_order_ties: bool) -> Vec<u32> {
        let mut graph = EliminationGraph::from_edges(5, &[(0, 1), (0, 2), (0, 4), (1, 2), (3, 4)]);
        let salt = vec![0; 5];
        let mut steps = ElimSteps::default();
        let exit = eliminate_min_degree(
            &mut graph,
            &salt,
            update_order_ties,
            steps.sink(),
            ElimStop::default(),
        );
        assert_eq!(exit, ElimExit::Complete);
        steps
            .rank_pairs
            .into_iter()
            .map(|(vertex, _)| vertex)
            .collect()
    }

    #[test]
    fn min_degree_requeues_a_neighbour_whose_degree_decreased() {
        let order = elimination_order(false);

        assert_eq!(&order[..2], [3, 4]);
    }

    #[test]
    fn update_order_ties_prefer_a_recently_exposed_leaf() {
        let order = elimination_order(true);

        assert_eq!(order, [3, 4, 1, 2, 0]);
    }
}
