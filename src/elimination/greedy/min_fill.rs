//! Min-fill elimination: repeatedly remove the active vertex whose removal adds
//! the fewest fill edges, ties broken by degree and then by the caller's salt.
//!
//! One instantiation of the greedy skeleton in `greedy`, and the portfolio's
//! main order. Fill is costly enough to be maintained rather than recomputed
//! per pop: a seeding scan measures every active vertex, then each elimination
//! re-scores every vertex whose neighbourhood or neighbour-pair edges changed.
//! Heap generations discard the older entries those updates replace.
//!
//! Past the soft deadline the run continues in cheap mode — neighbours are
//! re-pushed with fill 0, so the rest of the elimination pops in degree order.
//! What it emits is still a complete decomposition, but no longer a min-fill
//! one. The returned decomposition does not record that transition.

use std::time::Instant;

use super::deterministic::{AfterElim, ElimPolicy, Seeded, eliminate_greedy};
use super::*;
use crate::deadline::expired;

/// Heap entry ordered ascending by (fill, degree, salt). The `fill` field is
/// duplicated out of the key so the stale-snapshot check can compare it
/// against a live recomputed fill without destructuring the `Reverse` tuple.
#[derive(Eq, PartialEq)]
pub(super) struct HeapEntry {
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

impl HeapEntry {
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

ord_by_key!(HeapEntry);

impl ElimEntry for HeapEntry {
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
struct MinFill<'a> {
    heap: BinaryHeap<HeapEntry>,
    scratch: FillScratch,
    generation: Vec<u64>,
    score: Vec<u64>,
    affected: FillAffected,
    fill_edges: Vec<(u32, u32)>,
    salt: &'a [u32],
}

impl MinFill<'_> {
    fn deadline_outcome(&mut self, graph: &EliminationGraph) -> AfterElim {
        if graph.num_active > CHEAP_MODE_MAX_ACTIVE {
            AfterElim::Bail
        } else {
            AfterElim::EnterCheapMode
        }
    }
}

impl ElimPolicy for MinFill<'_> {
    type Entry = HeapEntry;

    const CHEAP_MODE: bool = true;
    const MAINTAIN_BITSET: bool = true;
    const ZERO_SCORE_IS_SIMPLICIAL: bool = true;

    fn heap(&mut self) -> &mut BinaryHeap<HeapEntry> {
        &mut self.heap
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

    fn entry_is_current(&self, entry: &HeapEntry) -> bool {
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

    fn eliminate_with_fill(&mut self, graph: &mut EliminationGraph, v: u32, nbrs: &[u32]) {
        self.affected.prepare_inside(graph, v, nbrs);
        graph.eliminate_with_nbrs_record_fill(v, nbrs, &mut self.fill_edges);
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

        if filled_neighbourhood {
            if !self
                .affected
                .collect_deltas(graph, nbrs, &self.fill_edges, deadline)
            {
                return self.deadline_outcome(graph);
            }
            while let Some((vertex, delta)) = self.affected.pop_delta() {
                if expired(deadline) {
                    self.affected.clear();
                    return self.deadline_outcome(graph);
                }
                debug_assert!(delta <= self.score[vertex as usize]);
                let score = self.score[vertex as usize].saturating_sub(delta);
                self.push(graph, vertex, score);
            }
        }

        // Checked inside this loop: a fill recount is superlinear in the
        // neighbourhood, so one update can otherwise overrun the deadline.
        for &vertex in nbrs {
            if expired(deadline) {
                return self.deadline_outcome(graph);
            }
            if graph.active[vertex as usize] {
                let live = self.scratch.fill_count_of(graph, vertex);
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
pub(crate) fn eliminate_min_fill(
    graph: &mut EliminationGraph,
    salt: &[u32],
    sink: ElimSink<'_>,
    stop: ElimStop,
) -> ElimExit {
    let n = graph.len();
    assert_eq!(salt.len(), n);
    let mut policy = MinFill {
        heap: BinaryHeap::with_capacity(n),
        scratch: FillScratch::new(n),
        generation: vec![0; n],
        score: vec![0; n],
        affected: FillAffected::new(n),
        fill_edges: Vec::new(),
        salt,
    };
    eliminate_greedy(&mut policy, graph, sink, stop)
}
