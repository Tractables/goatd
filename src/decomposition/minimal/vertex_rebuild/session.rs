use super::*;
use crate::decomposition::polishing::{self, Advance, Budget, Pause, Progress, Recipient};

/// A resumable vertex-reinsertion search with caller-owned acceptance.
///
/// Construction validates and preserves the supplied incumbent. One advance
/// step prepares its completion or attempts one vertex reconstruction. The
/// completion, scratch, vertex order and cursor survive pauses. A reconstruction
/// is indivisible at this interface; its internal loops honor the cooperative
/// deadline, then restore a complete valid decomposition before returning.
///
/// Proposals are offered before the width/mass filter used by [`improve`].
/// Rejecting a proposal moves to the next vertex; accepting it refreshes the
/// completion and order while preserving the cursor position. A full cycle
/// without acceptance exhausts the search. Arbitrary acceptance can revisit
/// incumbents, so the caller must keep its advance allocations finite.
pub struct Session<'g> {
    graph: &'g Graph,
    best: TreeDecomposition,
    best_quality: (u32, usize, Vec<u64>),
    shared: Option<(Vec<Vec<u32>>, SharedCompletion)>,
    order: Vec<u32>,
    custom_order: bool,
    cursor: u32,
    failed: u32,
    dirty: bool,
    progress: Progress,
    stats: Stats,
}

impl<'g> Session<'g> {
    /// Validate `tree` once and begin a search over `graph`.
    ///
    /// # Errors
    /// Returns an error if the decomposition is not valid for this graph.
    pub fn new(graph: &'g Graph, tree: TreeDecomposition) -> Result<Self, crate::Error> {
        let start = Instant::now();
        let units = crate::meter::units_spent();
        tree.validate(graph)?;
        let mut session = Self::trusted(graph, tree);
        session.progress.elapsed = start.elapsed();
        session.progress.setup_elapsed = session.progress.elapsed;
        session.progress.work_units = crate::meter::units_spent().saturating_sub(units);
        Ok(session)
    }

    pub(super) fn trusted(graph: &'g Graph, tree: TreeDecomposition) -> Self {
        Self {
            graph,
            best_quality: quality(&tree),
            best: tree,
            shared: None,
            order: Vec::new(),
            custom_order: false,
            cursor: 0,
            failed: 0,
            dirty: true,
            progress: Progress::default(),
            stats: Stats::default(),
        }
    }

    /// The accepted incumbent, which is always valid for the original graph.
    pub fn current(&self) -> &TreeDecomposition {
        &self.best
    }

    /// Work accumulated across all advance calls.
    pub fn progress(&self) -> Progress {
        self.progress
    }

    /// Override widest-bag-first order with a complete permutation of vertices.
    /// The permutation remains fixed after acceptance. Changing it restarts the
    /// cycle while retaining the incumbent and compatible completion storage.
    ///
    /// # Errors
    /// Returns an error for a missing, repeated or out-of-range vertex.
    pub fn set_vertex_order(&mut self, order: Vec<u32>) -> Result<(), crate::Error> {
        let n = self.graph.num_vertices() as usize;
        let mut seen = vec![false; n];
        if order.len() != n
            || order.iter().any(|&v| {
                let Some(slot) = seen.get_mut(v as usize) else {
                    return true;
                };
                std::mem::replace(slot, true)
            })
        {
            return Err(crate::Error::InvalidInput(
                "reinsertion order must contain each graph vertex once".into(),
            ));
        }
        self.order = order;
        self.custom_order = true;
        self.cursor = 0;
        self.failed = 0;
        Ok(())
    }

    /// Continue until a proposal, a budget pause or an exhausted cycle.
    pub fn advance(&mut self, budget: Budget) -> Advance<'_> {
        self.advance_inner(budget, None)
    }

    /// Consume the session and transfer its incumbent to another algorithm.
    /// Tree-dependent continuation state is discarded during this handoff.
    pub fn into_tree(self) -> TreeDecomposition {
        self.best
    }

    pub(super) fn advance_legacy(&mut self, deadline: Instant) -> Advance<'_> {
        self.advance_inner(Budget::new(u64::MAX), Some(deadline))
    }

    pub(super) fn finish_legacy(self) -> (TreeDecomposition, Stats) {
        (self.best, self.stats)
    }

    fn advance_inner(&mut self, budget: Budget, deadline: Option<Instant>) -> Advance<'_> {
        let mut slice = budget.begin();
        let _wall = slice.wall_guard();
        loop {
            if self.failed >= self.graph.num_vertices() {
                slice.record(&mut self.progress);
                return Advance::Exhausted;
            }
            let pause = slice
                .pause()
                .or_else(|| crate::deadline::expired(deadline).then_some(Pause::Deadline));
            if let Some(reason) = pause {
                slice.record(&mut self.progress);
                return Advance::Paused(reason);
            }
            if self.dirty {
                if let Some(deadline) = deadline {
                    let n = u64::from(self.graph.num_vertices());
                    let squares = self.best.bags().iter().fold(0u64, |sum, bag| {
                        let size = bag.vertices().len() as u64;
                        sum.saturating_add(size.saturating_mul(size))
                    });
                    let projected = (n.saturating_mul(n) / 64).saturating_add(squares);
                    if Duration::from_millis(crate::meter::milliseconds_for_units(projected))
                        > deadline.saturating_duration_since(crate::meter::now()) / 8
                    {
                        slice.record(&mut self.progress);
                        return Advance::Paused(Pause::SetupEstimate);
                    }
                }
                let start = Instant::now();
                let (_, shared) = self.shared.get_or_insert_with(|| {
                    (adjacency(self.graph), SharedCompletion::new(self.graph))
                });
                shared.complete(&self.best);
                if !self.custom_order {
                    widest_first(&self.best, &mut self.order);
                }
                self.dirty = false;
                self.stats.rounds += 1;
                self.progress.setup_elapsed += start.elapsed();
                slice.step();
                continue;
            }
            let vertex = self.order[self.cursor as usize];
            self.cursor = (self.cursor + 1) % self.graph.num_vertices();
            self.progress.attempted += 1;
            let (adjacency, shared) = self
                .shared
                .as_mut()
                .expect("completion prepared before reconstruction");
            let candidate = rebuild(
                self.graph,
                &adjacency[vertex as usize],
                shared,
                &self.best,
                vertex,
                deadline,
            );
            slice.step();
            if let Some(candidate) = candidate {
                self.stats.tried += 1;
                self.progress.proposed += 1;
                let recommended = quality(&candidate) < self.best_quality;
                slice.record(&mut self.progress);
                return polishing::proposal(self, candidate, recommended);
            }
            self.failed += 1;
        }
    }
}

impl Recipient for Session<'_> {
    fn current(&self) -> &TreeDecomposition {
        &self.best
    }

    fn accept(&mut self, candidate: TreeDecomposition) {
        self.best_quality = quality(&candidate);
        self.best = candidate;
        self.progress.accepted += 1;
        self.stats.improved += 1;
        self.failed = 0;
        self.dirty = true;
    }

    fn reject(&mut self) {
        self.failed += 1;
    }
}
