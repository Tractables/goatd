//! Min-fill elimination: repeatedly remove the active vertex whose removal adds
//! the fewest fill edges, ties broken by degree and then by the caller's salt.
//!
//! One instantiation of the greedy skeleton in `greedy`, and the portfolio's
//! main order. Fill is costly enough to be maintained rather than recomputed
//! per pop: a seeding scan measures every active vertex, then each elimination
//! updates every vertex whose neighbourhood or neighbour-pair edges changed,
//! from what `FillAffected` read before the elimination. Heap generations
//! discard the older entries those updates replace.
//!
//! Past the soft deadline the run continues in cheap mode — neighbours are
//! re-pushed with fill 0, so the rest of the elimination pops in degree order.
//! What it emits is still a complete decomposition, but no longer a min-fill
//! one. The returned decomposition does not record that transition.

use std::time::Instant;

use super::deterministic::{AfterElim, ElimPolicy, Seeded, eliminate_greedy};
use super::*;
use crate::deadline::expired;

/// Heap entry ordered by fill or fill/degree, then degree and salt. The `fill` field is
/// duplicated out of the key so the stale-snapshot check can compare it
/// against a live recomputed fill without destructuring the `Reverse` tuple.
#[derive(Eq, PartialEq)]
pub(super) struct HeapEntry<const RELATIVE: bool> {
    pub key: (
        Reverse<u64>,
        Reverse<usize>,
        Reverse<u32>,
        Reverse<u32>,
        u64,
    ),
    pub vertex: u32,
    pub fill: u64,
    pub generation: u64,
}

impl<const RELATIVE: bool> HeapEntry<RELATIVE> {
    pub(super) fn new(fill: u64, degree: usize, salt: u32, v: u32, generation: u64) -> Self {
        HeapEntry {
            key: (
                Reverse(fill),
                Reverse(degree),
                Reverse(salt),
                Reverse(v),
                generation,
            ),
            vertex: v,
            fill,
            generation,
        }
    }
}

impl<const RELATIVE: bool> Ord for HeapEntry<RELATIVE> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if RELATIVE {
            let left = u128::from(self.fill) * other.key.1.0.max(1) as u128;
            let right = u128::from(other.fill) * self.key.1.0.max(1) as u128;
            right.cmp(&left).then_with(|| {
                (self.key.1, self.key.2, self.key.3, self.key.4).cmp(&(
                    other.key.1,
                    other.key.2,
                    other.key.3,
                    other.key.4,
                ))
            })
        } else {
            self.key.cmp(&other.key)
        }
    }
}

impl<const RELATIVE: bool> PartialOrd for HeapEntry<RELATIVE> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<const RELATIVE: bool> ElimEntry for HeapEntry<RELATIVE> {
    fn vertex(&self) -> u32 {
        self.vertex
    }
    fn snapshot(&self) -> u64 {
        self.fill
    }
}

/// Fill counts for every active vertex, leaving 0 for anything the scan did
/// not reach. The core measures everything up front and only then builds the
/// heap, so that a scan cut short by the deadline still yields an entry per
/// active vertex.
///
/// `deadline`/`hard_deadline` bound the scan itself, which on a very large
/// graph can consume most of a candidate's budget on its own: `Bailed` means the
/// hard deadline passed and nothing should be eliminated, `CheapMode` means
/// the soft deadline cut the scan short and the run starts with incomplete
/// scores.
fn scan_fill(
    scratch: &mut FillScratch,
    graph: &EliminationGraph,
    fill_count: &mut [u64],
    deadline: Option<Instant>,
    hard_deadline: Option<Instant>,
) -> Seeded {
    let mut pacer = DeadlinePacer::new();
    for (v, slot) in fill_count.iter_mut().enumerate() {
        if !graph.active[v] {
            continue;
        }
        *slot = scratch.fill_count_of(graph, v as u32);
        // Paced by what the scores have cost, not by how many were taken: one
        // score on a dense residual runs into milliseconds, and 64 of them used
        // to carry the scan seconds past the hard deadline.
        if pacer.due() {
            if expired(hard_deadline) {
                return Seeded::Bailed;
            }
            if expired(deadline) {
                return Seeded::CheapMode;
            }
        }
    }
    Seeded::Ready
}

/// Greedy min-fill: rank by the number of fill edges eliminating a vertex
/// would add, breaking ties by degree and then by salt.
struct MinFill<'a, const RELATIVE: bool> {
    heap: BinaryHeap<HeapEntry<RELATIVE>>,
    scratch: FillScratch,
    generation: Vec<u64>,
    score: Vec<u64>,
    affected: FillAffected,
    /// Whether `affected` holds what the last elimination disturbed. A
    /// `prepare` that reaches the deadline part way through clears itself and
    /// leaves this false.
    prepared: bool,
    salt: &'a [u32],
}

impl<const RELATIVE: bool> MinFill<'_, RELATIVE> {
    fn deadline_outcome(&mut self, graph: &EliminationGraph) -> AfterElim {
        if graph.num_active > CHEAP_MODE_MAX_ACTIVE {
            AfterElim::Bail
        } else {
            AfterElim::EnterCheapMode
        }
    }
}

impl<const RELATIVE: bool> ElimPolicy for MinFill<'_, RELATIVE> {
    type Entry = HeapEntry<RELATIVE>;

    const CHEAP_MODE: bool = true;
    const MAINTAIN_BITSET: bool = true;
    const ZERO_SCORE_IS_SIMPLICIAL: bool = true;

    fn pop(&mut self) -> Option<HeapEntry<RELATIVE>> {
        self.heap.pop()
    }

    fn push(&mut self, graph: &EliminationGraph, v: u32, score: u64) {
        self.score[v as usize] = score;
        let generation = self.generation[v as usize].wrapping_add(1);
        self.generation[v as usize] = generation;
        self.heap.push(HeapEntry::new(
            score,
            graph.degree(v),
            self.salt[v as usize],
            v,
            generation,
        ));
    }

    fn entry_is_current(&self, entry: &HeapEntry<RELATIVE>) -> bool {
        self.generation[entry.vertex as usize] == entry.generation
    }

    fn live_score(&mut self, graph: &EliminationGraph, v: u32) -> u64 {
        self.scratch.fill_count_of(graph, v)
    }

    fn seed(
        &mut self,
        graph: &mut EliminationGraph,
        deadline: Option<Instant>,
        hard_deadline: Option<Instant>,
    ) -> Seeded {
        let mut fill_count: Vec<u64> = vec![0; graph.len()];
        let outcome = scan_fill(
            &mut self.scratch,
            graph,
            &mut fill_count,
            deadline,
            hard_deadline,
        );
        if matches!(outcome, Seeded::Bailed) {
            return outcome;
        }
        for (v, &fill) in fill_count.iter().enumerate() {
            if graph.active[v] {
                self.push(graph, v as u32, fill);
            }
        }
        outcome
    }

    fn rescore_on_pop(&mut self, _graph: &EliminationGraph, _v: u32) -> Option<u64> {
        None
    }

    fn eliminate_with_fill(
        &mut self,
        graph: &mut EliminationGraph,
        v: u32,
        nbrs: &[u32],
        deadline: Option<Instant>,
    ) {
        self.prepared = self.affected.prepare(graph, v, nbrs, true, deadline);
        if self.prepared {
            graph.eliminate_prepared(v, nbrs, &self.affected.fill_edges());
        } else {
            graph.eliminate_with_nbrs(v, nbrs);
        }
    }

    fn eliminate_simplicial(&mut self, graph: &mut EliminationGraph, v: u32, nbrs: &[u32]) {
        self.prepared = self.affected.prepare(graph, v, nbrs, false, None);
        graph.remove_without_fill_nbrs(v, nbrs);
    }

    fn after_eliminate(
        &mut self,
        graph: &EliminationGraph,
        nbrs: &[u32],
        cheap_mode: bool,
        deadline: Option<Instant>,
        filled_neighbourhood: bool,
    ) -> AfterElim {
        if cheap_mode {
            // Fill accuracy is already abandoned: re-push each live neighbour
            // with a zero fill so the rest of the run pops in min-degree order.
            for &u in nbrs {
                if graph.active[u as usize] {
                    self.push(graph, u, 0);
                }
            }
            return AfterElim::Continue;
        }

        if !self.prepared {
            return self.deadline_outcome(graph);
        }
        if filled_neighbourhood {
            // Applying one delta is a bucket move, so this loop reads the
            // deadline on the pacer's stride.
            let mut pacer = DeadlinePacer::new();
            while let Some((vertex, delta)) = self.affected.pop_delta(graph) {
                if pacer.due() && expired(deadline) {
                    self.affected.clear();
                    return self.deadline_outcome(graph);
                }
                debug_assert!(delta <= self.score[vertex as usize]);
                let score = self.score[vertex as usize].saturating_sub(delta);
                self.push(graph, vertex, score);
            }
        }

        for &vertex in nbrs {
            if graph.active[vertex as usize] {
                let live = self
                    .affected
                    .neighbour_fill(graph, vertex, self.score[vertex as usize]);
                self.push(graph, vertex, live);
            }
        }
        AfterElim::Continue
    }
}

/// Eliminate every remaining active vertex from `graph` using the greedy
/// min-fill rule, recording the emitted bags (first vertex = eliminated, rest
/// = live neighbours) into `sink`.
///
/// `salt[v]` breaks (fill, degree) ties; `0` salt gives deterministic
/// vertex-id order, random values give diversification across seeds.
pub(crate) fn eliminate_min_fill<const RELATIVE: bool>(
    graph: &mut EliminationGraph,
    salt: &[u32],
    sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    let n = graph.len();
    assert_eq!(salt.len(), n);
    let mut policy = MinFill::<RELATIVE> {
        heap: BinaryHeap::with_capacity(n),
        scratch: FillScratch::new(n),
        generation: vec![0; n],
        score: vec![0; n],
        affected: FillAffected::new(n),
        prepared: false,
        salt,
    };
    eliminate_greedy(&mut policy, graph, sink, stop)
}
