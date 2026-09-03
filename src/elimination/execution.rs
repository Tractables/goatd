//! Limits and output records shared by elimination-based constructions.

use std::time::Instant;

use super::graph::EliminationGraph;

/// Number of inexpensive loop iterations between deadline reads.
pub(super) const DEADLINE_CHECK_STRIDE: u32 = 64;

/// Conditions that can change or stop an elimination run.
///
/// Deterministic greedy orders use `soft_deadline` to switch to cheaper
/// scoring. `hard_deadline` stops every order. `width_bound` lets a portfolio
/// stop a candidate once it cannot improve the best width already found.
#[derive(Clone, Copy, Default)]
pub(crate) struct ElimStop {
    pub(crate) soft_deadline: Option<Instant>,
    pub(crate) hard_deadline: Option<Instant>,
    pub(crate) width_bound: Option<u32>,
}

/// Which of the two cutoffs stopped a run.
///
/// The distinction is the caller's, not the core's: a core that stops at the
/// soft cutoff has spent the construction budget it was given, while the
/// portfolio around it still has hard-deadline time for another candidate. A
/// core that stops at the hard cutoff leaves no time for anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cutoff {
    Soft,
    Hard,
}

/// How an elimination run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ElimExit {
    Complete,
    DeadlineReached(Cutoff),
    WidthLimitExceeded,
}

/// Whether a newly emitted bag is already too wide to improve `bound`.
#[inline]
pub(super) fn exceeds_width_bound(bag_len: usize, bound: Option<u32>) -> bool {
    matches!(bound, Some(b) if bag_len > b as usize + 1)
}

/// Bags and elimination ranks produced by one or more consecutive phases.
///
/// Appending through [`ElimSteps::sink`] keeps the two records synchronized
/// and continues step numbering across preprocessing and order construction.
#[derive(Clone, Default)]
pub(super) struct ElimSteps {
    /// Each entry is the eliminated vertex followed by its live neighbours.
    pub(super) bags: Vec<Vec<u32>>,
    /// `(vertex, step)` for every eliminated vertex.
    pub(super) rank_pairs: Vec<(u32, usize)>,
}

impl ElimSteps {
    pub(super) fn sink(&mut self) -> ElimSink<'_> {
        let start_step = self.bags.len();
        ElimSink::new(&mut self.bags, &mut self.rank_pairs, start_step)
    }

    /// Append component-local steps after translating vertices through `comp`.
    pub(super) fn append_reindexed(self, comp: &[u32], bags: &mut Vec<Vec<u32>>, rank: &mut [u32]) {
        let base = bags.len();
        for mut bag in self.bags {
            for v in &mut bag {
                *v = comp[*v as usize];
            }
            bags.push(bag);
        }
        for (local_v, step) in self.rank_pairs {
            rank[comp[local_v as usize] as usize] = (base + step) as u32;
        }
    }
}

/// Appends one bag and its matching elimination rank as a single operation.
pub(super) struct ElimSink<'a> {
    bags: &'a mut Vec<Vec<u32>>,
    ranks: &'a mut Vec<(u32, usize)>,
    step: usize,
}

impl<'a> ElimSink<'a> {
    pub(super) fn new(
        bags: &'a mut Vec<Vec<u32>>,
        ranks: &'a mut Vec<(u32, usize)>,
        start_step: usize,
    ) -> Self {
        Self {
            bags,
            ranks,
            step: start_step,
        }
    }

    #[inline]
    pub(super) fn record(&mut self, vertex: u32, bag: Vec<u32>) {
        self.bags.push(bag);
        self.ranks.push((vertex, self.step));
        self.step += 1;
    }
}

/// Return the active vertices in their original index order.
pub(super) fn active_vertices(graph: &EliminationGraph) -> Vec<u32> {
    (0..graph.len() as u32)
        .filter(|&v| graph.active[v as usize])
        .collect()
}
