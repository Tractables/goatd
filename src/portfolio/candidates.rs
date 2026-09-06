use crate::TreeDecomposition;
use crate::decomposition::SubsumedBagCompaction;
use crate::elimination::engine::OrderRun;
use crate::elimination::execution::Cutoff;

use super::trace::{CandidateOrigin, CandidateOutcome};

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

/// One decomposition the portfolio produced, with the candidate that made it.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// The decomposition, bags contained in an adjacent bag contracted.
    pub decomposition: TreeDecomposition,
    /// Which candidate of the schedule produced it.
    pub origin: CandidateOrigin,
}

/// A produced decomposition as the set holds it: uncompacted, with the plan
/// that compacts it and the key the compacted form sorts on.
struct Retained {
    decomposition: TreeDecomposition,
    origin: CandidateOrigin,
    compaction: SubsumedBagCompaction,
    quality_key: (u32, usize),
}

/// Produced decompositions and the incumbent width handed to later
/// elimination candidates.
pub(super) struct CandidateSet {
    retained: Vec<Retained>,
    best_width: Option<u32>,
    best_quality_key: Option<(u32, usize)>,
    retain_only_best: bool,
}

impl CandidateSet {
    pub(super) fn all(capacity: usize) -> Self {
        Self {
            retained: Vec::with_capacity(capacity),
            best_width: None,
            best_quality_key: None,
            retain_only_best: false,
        }
    }

    pub(super) fn best_only() -> Self {
        Self {
            retained: Vec::with_capacity(1),
            best_width: None,
            best_quality_key: None,
            retain_only_best: true,
        }
    }

    pub(super) fn best_width(&self) -> Option<u32> {
        self.best_width
    }

    pub(super) fn is_empty(&self) -> bool {
        self.retained.is_empty()
    }

    /// The decomposition the set would return, uncompacted, or `None` before
    /// any candidate has produced one.
    pub(super) fn best(&self) -> Option<&TreeDecomposition> {
        self.retained
            .iter()
            .min_by_key(|retained| retained.quality_key)
            .map(|retained| &retained.decomposition)
    }

    /// Record a decomposition and report what it is worth. `best` says whether
    /// the set would now return this one. Every mode compares on the key of the
    /// compacted decomposition, width first and then the total bag size once
    /// the bags contained in an adjacent bag are contracted, so the
    /// all-candidates sort and the best-only retention agree on the winner.
    pub(super) fn push(
        &mut self,
        decomposition: TreeDecomposition,
        origin: CandidateOrigin,
    ) -> CandidateOutcome {
        let (width, total_bag_size) = decomposition.quality_key();
        self.best_width = Some(self.best_width.map_or(width, |best| best.min(width)));
        // A candidate wider than the incumbent cannot win in either mode, and
        // a plan it will not be sorted on is not worth computing; in
        // all-candidates mode every decomposition is returned and needs one.
        let contender = self.best_quality_key.is_none_or(|best| width <= best.0);
        let mut best = false;
        if !self.retain_only_best || contender {
            let compaction = decomposition.subsumed_bag_compaction();
            let quality_key = (width, compaction.total_bag_size());
            best = self
                .best_quality_key
                .is_none_or(|incumbent| quality_key < incumbent);
            if best {
                self.best_quality_key = Some(quality_key);
            }
            if best || !self.retain_only_best {
                if self.retain_only_best {
                    self.retained.clear();
                }
                self.retained.push(Retained {
                    decomposition,
                    origin,
                    compaction,
                    quality_key,
                });
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
    pub(super) fn record_elimination(
        &mut self,
        run: OrderRun,
        origin: CandidateOrigin,
    ) -> (CandidateOutcome, ScheduleStop) {
        match run {
            OrderRun::Completed(decomposition) => {
                (self.push(decomposition, origin), ScheduleStop::Continue)
            }
            OrderRun::CompletedAtDeadline(Cutoff::Soft, decomposition) => {
                (self.push(decomposition, origin), ScheduleStop::Continue)
            }
            OrderRun::CompletedAtDeadline(Cutoff::Hard, decomposition) => {
                self.push(decomposition, origin);
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

    /// Every retained decomposition, compacted, with its origin, sorted
    /// ascending by width and then total bag size with ties kept in candidate
    /// order. In best-only mode this is the winner alone.
    pub(super) fn into_candidates(self) -> Vec<Candidate> {
        let mut retained = self.retained;
        retained.sort_by_key(|retained| retained.quality_key);
        retained
            .into_iter()
            .map(|retained| Candidate {
                decomposition: retained.compaction.apply(retained.decomposition),
                origin: retained.origin,
            })
            .collect()
    }

    /// [`Self::into_candidates`] without the origins.
    pub(super) fn into_decompositions(self) -> Vec<TreeDecomposition> {
        self.into_candidates()
            .into_iter()
            .map(|candidate| candidate.decomposition)
            .collect()
    }
}
