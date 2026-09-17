//! Budgets and proposals for resumable decomposition refinement.

use std::cell::OnceCell;
use std::time::{Duration, Instant};

use super::TreeDecomposition;

/// Limits for one advance of a refinement session.
///
/// Steps count the session's documented operations, independently of machine
/// speed. The optional deadline always uses real elapsed time, even when the
/// construction work meter is armed. Operations are cooperative: an individual
/// reconstruction or cutter advance can finish after the deadline.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct Budget {
    steps: u64,
    deadline: Option<Instant>,
}

impl Budget {
    /// Allow at most `steps` operations. Zero performs no search or lazy setup.
    pub const fn new(steps: u64) -> Self {
        Self {
            steps,
            deadline: None,
        }
    }

    /// Also stop at an absolute real-time deadline.
    pub const fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    pub(crate) fn begin(self) -> Slice {
        Slice {
            budget: self,
            used: 0,
            start: Instant::now(),
            units: crate::meter::units_spent(),
        }
    }
}

/// Why an advance paused. A later call can supply another allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pause {
    /// The allotted operations were consumed.
    Steps,
    /// The budget's deadline was reached.
    Deadline,
    /// The caller requested cancellation through the stop flag.
    Stopped,
    /// The session had to rebuild its shared state before it could propose
    /// anything, and the estimated cost of that rebuild did not fit the time
    /// left. Only a budget carrying a deadline can see this: with no deadline
    /// there is no time left to compare the estimate against.
    SetupEstimate,
}

/// Cumulative search measurements, excluding time spent by the caller scoring
/// proposals. Constructor validation is included in `elapsed` and `setup_elapsed`.
/// Accessing, accepting and extracting trees is not counted. Measure the whole
/// session lifecycle separately to include those costs in a pipeline budget.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct Progress {
    /// Completed scheduling operations, including lazy setup operations.
    pub steps: u64,
    /// Reconstruction or separator attempts started.
    pub attempted: u64,
    /// Valid decomposition proposals offered to the caller.
    pub proposed: u64,
    /// Proposals accepted by the caller.
    pub accepted: u64,
    /// Graph work charged by the existing construction meter.
    pub work_units: u64,
    /// Real time in constructor and advance calls.
    pub elapsed: Duration,
    /// Real time spent validating or preparing the search.
    pub setup_elapsed: Duration,
}

/// The next proposal, a resumable pause, or a completed search.
#[must_use]
#[non_exhaustive]
pub enum Advance<'a> {
    /// A valid candidate awaiting the caller's decision.
    Proposal(Proposal<'a>),
    /// The session retains its cursor and scratch for another advance.
    Paused(Pause),
    /// The configured search has no further work.
    Exhausted,
}

/// A candidate tied to the session that produced it.
///
/// Both trees use the complete original graph's vertex space. Accepting commits
/// the candidate; rejecting or dropping preserves the incumbent and advances
/// the search. The mutable session borrow prevents another advance or a kernel
/// handoff while this decision is outstanding.
#[must_use]
pub struct Proposal<'a> {
    recipient: &'a mut dyn Recipient,
    candidate: Option<TreeDecomposition>,
    complete: OnceCell<TreeDecomposition>,
    recommended: bool,
}

impl Proposal<'_> {
    /// The incumbent before this proposal.
    pub fn current(&self) -> &TreeDecomposition {
        self.recipient.current()
    }

    /// The complete candidate. Recursive refinement materializes it lazily.
    pub fn candidate(&self) -> &TreeDecomposition {
        let candidate = self
            .candidate
            .as_ref()
            .expect("an outstanding proposal owns its candidate");
        if self.recipient.candidate_is_complete() {
            candidate
        } else {
            self.complete
                .get_or_init(|| self.recipient.complete(candidate))
        }
    }

    /// Commit this proposal to its originating session.
    pub fn accept(mut self) {
        let candidate = self
            .candidate
            .take()
            .expect("a proposal can be accepted once");
        self.recipient.accept(candidate);
    }

    /// Preserve the incumbent and continue past this proposal.
    pub fn reject(self) {}

    pub(crate) fn recommended(&self) -> bool {
        self.recommended
    }
}

impl Drop for Proposal<'_> {
    fn drop(&mut self) {
        if self.candidate.is_some() {
            self.recipient.reject();
        }
    }
}

pub(crate) trait Recipient {
    fn current(&self) -> &TreeDecomposition;
    fn candidate_is_complete(&self) -> bool {
        true
    }
    fn complete(&self, candidate: &TreeDecomposition) -> TreeDecomposition {
        candidate.clone()
    }
    fn accept(&mut self, candidate: TreeDecomposition);
    fn reject(&mut self);
}

pub(crate) fn proposal(
    recipient: &mut dyn Recipient,
    candidate: TreeDecomposition,
    recommended: bool,
) -> Advance<'_> {
    Advance::Proposal(new_proposal(recipient, candidate, recommended))
}

pub(crate) fn new_proposal(
    recipient: &mut dyn Recipient,
    candidate: TreeDecomposition,
    recommended: bool,
) -> Proposal<'_> {
    Proposal {
        recipient,
        candidate: Some(candidate),
        complete: OnceCell::new(),
        recommended,
    }
}

pub(crate) struct Slice {
    budget: Budget,
    used: u64,
    start: Instant,
    units: u64,
}

impl Slice {
    pub(crate) fn pause(&self) -> Option<Pause> {
        if crate::stop::requested() {
            Some(Pause::Stopped)
        } else if self
            .budget
            .deadline
            .is_some_and(|end| Instant::now() >= end)
        {
            Some(Pause::Deadline)
        } else if self.used >= self.budget.steps {
            Some(Pause::Steps)
        } else {
            None
        }
    }

    pub(crate) fn step(&mut self) {
        self.used += 1;
    }

    pub(crate) fn wall_guard(&self) -> crate::deadline::WallGuard {
        crate::deadline::WallGuard::new(self.budget.deadline)
    }

    /// The deadline the caller set on the budget, for the work inside the
    /// slice: `pause` only bounds the loop around it.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.budget.deadline
    }

    pub(crate) fn record(self, progress: &mut Progress) {
        progress.steps = progress.steps.saturating_add(self.used);
        progress.work_units = progress
            .work_units
            .saturating_add(crate::meter::units_spent().saturating_sub(self.units));
        progress.elapsed += self.start.elapsed();
    }
}
