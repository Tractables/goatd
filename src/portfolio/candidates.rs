use crate::TreeDecomposition;
use crate::decomposition::SubsumedBagCompaction;
use crate::elimination::engine::OrderRun;
use crate::elimination::execution::Cutoff;

use super::trace::CandidateOutcome;

/// Whether the portfolio may start another candidate after this one.
///
/// A candidate that stopped at the soft cutoff has spent the construction
/// budget, not the portfolio's whole window: the schedule carries on and the
/// trailing FlowCutter slot still runs. Only the hard cutoff ends the
/// portfolio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScheduleStop {
    Continue,
    HardDeadline,
}

/// Produced decompositions and the incumbent width handed to later
/// elimination candidates.
pub(super) struct CandidateSet {
    decompositions: Vec<TreeDecomposition>,
    best_width: Option<u32>,
    best_quality_key: Option<(u32, usize)>,
    best_compaction: Option<SubsumedBagCompaction>,
    retain_only_best: bool,
}

impl CandidateSet {
    pub(super) fn all(capacity: usize) -> Self {
        Self {
            decompositions: Vec::with_capacity(capacity),
            best_width: None,
            best_quality_key: None,
            best_compaction: None,
            retain_only_best: false,
        }
    }

    pub(super) fn best_only() -> Self {
        Self {
            decompositions: Vec::with_capacity(1),
            best_width: None,
            best_quality_key: None,
            best_compaction: None,
            retain_only_best: true,
        }
    }

    pub(super) fn best_width(&self) -> Option<u32> {
        self.best_width
    }

    pub(super) fn is_empty(&self) -> bool {
        self.decompositions.is_empty()
    }

    /// The decomposition the set would return, or `None` before any candidate
    /// has produced one.
    pub(super) fn best(&self) -> Option<&TreeDecomposition> {
        self.decompositions
            .iter()
            .min_by_key(|decomposition| decomposition.quality_key())
    }

    /// Record a decomposition and report what it is worth. `best` says whether
    /// the set would now return this one: in best-only mode that is whether it
    /// was retained, and in all-candidates mode whether its quality key is the
    /// smallest so far, which is what the final sort picks.
    pub(super) fn push(&mut self, decomposition: TreeDecomposition) -> CandidateOutcome {
        let (width, total_bag_size) = decomposition.quality_key();
        self.best_width = Some(self.best_width.map_or(width, |best| best.min(width)));
        let mut best = false;
        if !self.retain_only_best {
            best = self
                .best_quality_key
                .is_none_or(|incumbent| (width, total_bag_size) < incumbent);
            if best {
                self.best_quality_key = Some((width, total_bag_size));
            }
            self.decompositions.push(decomposition);
        } else if self.best_quality_key.is_none_or(|best| width <= best.0) {
            let compaction = decomposition.subsumed_bag_compaction();
            let quality_key = (width, compaction.total_bag_size());
            if self.best_quality_key.is_none_or(|best| quality_key < best) {
                best = true;
                self.decompositions.clear();
                self.decompositions.push(decomposition);
                self.best_quality_key = Some(quality_key);
                self.best_compaction = Some(compaction);
            }
        }
        CandidateOutcome::Produced {
            width,
            total_bag_size,
            best,
        }
    }

    /// Record what a candidate returned, and say whether the schedule goes on.
    ///
    /// A soft-cutoff stop that completed its residual is reported as a
    /// produced decomposition, because that is what it is: the width and total
    /// bag size are the ones the portfolio now holds. Only the hard cutoff
    /// ends the schedule.
    pub(super) fn record_elimination(&mut self, run: OrderRun) -> (CandidateOutcome, ScheduleStop) {
        match run {
            OrderRun::Completed(decomposition) => {
                (self.push(decomposition), ScheduleStop::Continue)
            }
            OrderRun::CompletedAtDeadline(Cutoff::Soft, decomposition) => {
                (self.push(decomposition), ScheduleStop::Continue)
            }
            OrderRun::CompletedAtDeadline(Cutoff::Hard, decomposition) => {
                self.push(decomposition);
                (
                    CandidateOutcome::DeadlineReached,
                    ScheduleStop::HardDeadline,
                )
            }
            OrderRun::DeadlineAborted(Cutoff::Soft) => {
                (CandidateOutcome::DeadlineReached, ScheduleStop::Continue)
            }
            OrderRun::DeadlineAborted(Cutoff::Hard) => (
                CandidateOutcome::DeadlineReached,
                ScheduleStop::HardDeadline,
            ),
            OrderRun::WidthAborted => (CandidateOutcome::WidthAborted, ScheduleStop::Continue),
        }
    }

    pub(super) fn into_decompositions(mut self) -> Vec<TreeDecomposition> {
        if self.retain_only_best {
            let Some(decomposition) = self.decompositions.pop() else {
                return Vec::new();
            };
            let compaction = self
                .best_compaction
                .take()
                .expect("a retained candidate has a compaction plan");
            return vec![compaction.apply(decomposition)];
        }
        self.decompositions
    }
}
