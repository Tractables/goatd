use crate::TreeDecomposition;
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
}

impl CandidateSet {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            decompositions: Vec::with_capacity(capacity),
            best_width: None,
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
        self.decompositions.push(decomposition);
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

    pub(super) fn into_decompositions(self) -> Vec<TreeDecomposition> {
        self.decompositions
    }
}
