use crate::TreeDecomposition;
use crate::decomposition::SubsumedBagCompaction;
use crate::elimination::engine::OrderRun;

/// What one portfolio candidate's elimination left behind.
pub(super) enum CandidateOutcome {
    /// A decomposition, recorded and folded into the best width so far.
    Produced,
    /// A bag passed the width bound, so nothing usable came back. That bound
    /// comes from a candidate that already produced one, so a winner exists.
    WidthAborted,
    /// The hard deadline was reached, with or without a completed residual.
    /// Later candidates should not start.
    DeadlineReached,
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

    pub(super) fn push(&mut self, decomposition: TreeDecomposition) {
        let width = decomposition.treewidth();
        self.best_width = Some(self.best_width.map_or(width, |best| best.min(width)));
        if !self.retain_only_best {
            self.decompositions.push(decomposition);
        } else {
            if self.best_quality_key.is_some_and(|best| width > best.0) {
                return;
            }
            let compaction = decomposition.subsumed_bag_compaction();
            let quality_key = (width, compaction.total_bag_size());
            if self.best_quality_key.is_none_or(|best| quality_key < best) {
                self.decompositions.clear();
                self.decompositions.push(decomposition);
                self.best_quality_key = Some(quality_key);
                self.best_compaction = Some(compaction);
            }
        }
    }

    pub(super) fn record_elimination(&mut self, run: OrderRun) -> CandidateOutcome {
        match run {
            OrderRun::Completed(decomposition) => {
                self.push(decomposition);
                CandidateOutcome::Produced
            }
            OrderRun::CompletedAtDeadline(decomposition) => {
                self.push(decomposition);
                CandidateOutcome::DeadlineReached
            }
            OrderRun::DeadlineAborted => CandidateOutcome::DeadlineReached,
            OrderRun::WidthAborted => CandidateOutcome::WidthAborted,
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
