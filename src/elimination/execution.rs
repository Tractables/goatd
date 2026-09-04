//! Limits and output records shared by elimination-based constructions.

use std::time::Instant;

use super::graph::EliminationGraph;

/// Ceiling on the number of loop iterations between deadline reads, for a loop
/// whose work the meter is not charged for.
const DEADLINE_CHECK_STRIDE: u32 = 64;

/// Work that may pass between deadline reads, in the units the meter charges:
/// one millisecond's worth at the meter's rate.
const DEADLINE_CHECK_UNITS: u64 = crate::meter::UNITS_PER_MS;

/// Decides when a loop should read the deadline, from the work charged since
/// the last read.
///
/// A count of iterations is the wrong interval where one iteration can cost as
/// much as thousands of others: scoring one vertex of a dense residual for
/// min-fill runs into milliseconds, and 64 of those carried runs seconds past
/// their hard deadline. The work every operation charges [`crate::meter`] is
/// already the library's own measure of cost, so the pacer asks for the clock
/// once a millisecond's worth of it has been charged. The iteration count stays
/// as a ceiling, so a loop that charges nothing still reads the clock as often
/// as it used to.
pub(super) struct DeadlinePacer {
    steps: u32,
    mark: u64,
}

impl DeadlinePacer {
    pub(super) fn new() -> Self {
        Self {
            steps: 0,
            mark: crate::meter::units_spent(),
        }
    }

    /// Count one iteration and report whether the deadline should be read.
    #[inline]
    pub(super) fn due(&mut self) -> bool {
        self.steps += 1;
        let spent = crate::meter::units_spent();
        if self.steps >= DEADLINE_CHECK_STRIDE
            || spent.saturating_sub(self.mark) >= DEADLINE_CHECK_UNITS
        {
            self.steps = 0;
            self.mark = spent;
            return true;
        }
        false
    }
}

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
    /// Stop a candidate as soon as a bag reaches `width_bound` rather than
    /// passes it, so only a strictly narrower run finishes.
    pub(crate) abort_on_tie: bool,
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
///
/// A bag of `bag_len` vertices carries width `bag_len - 1`. Without
/// `abort_on_tie` the run continues while it can still match `bound`, so a
/// candidate that ties the incumbent finishes and can win on total bag size.
/// With it the run stops as soon as a bag reaches `bound`, so only a strictly
/// narrower candidate finishes.
#[inline]
pub(super) fn exceeds_width_bound(bag_len: usize, bound: Option<u32>, abort_on_tie: bool) -> bool {
    match bound {
        Some(bound) if abort_on_tie => bag_len > bound as usize,
        Some(bound) => bag_len > bound as usize + 1,
        None => false,
    }
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
