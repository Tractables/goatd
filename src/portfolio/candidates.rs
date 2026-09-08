use std::collections::HashSet;

use crate::TreeDecomposition;
use crate::decomposition::{BagPool, BagPoolLimits, SubsumedBagCompaction};
use crate::elimination::engine::OrderRun;
use crate::elimination::execution::Cutoff;

use super::trace::{CandidateOrigin, CandidateOutcome, Shape};

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
    /// Whether a produced candidate carries its shape numbers. Only a traced
    /// run has anywhere to report them, and they cost a pass over the bags.
    report_shape: bool,
    /// The bags of every candidate, for the recombination stage. `None` where
    /// no stage is going to read them, which is the ordinary case; keeping
    /// them costs a sorted copy of each bag.
    pool: Option<BagPool>,
}

impl CandidateSet {
    pub(super) fn all(capacity: usize) -> Self {
        Self {
            retained: Vec::with_capacity(capacity),
            best_width: None,
            best_quality_key: None,
            retain_only_best: false,
            report_shape: false,
            pool: None,
        }
    }

    pub(super) fn best_only() -> Self {
        Self {
            retained: Vec::with_capacity(1),
            best_width: None,
            best_quality_key: None,
            retain_only_best: true,
            report_shape: false,
            pool: None,
        }
    }

    /// Report each produced candidate's shape numbers, or not. A run whose
    /// trace sink discards everything does not compute them.
    pub(super) fn reporting_shape(mut self, on: bool) -> Self {
        self.report_shape = on;
        self
    }

    /// Keep the bags of every candidate the set is given, under `limits`.
    pub(super) fn collecting_bags(mut self, limits: BagPoolLimits) -> Self {
        self.pool = Some(BagPool::new(limits));
        self
    }

    /// The collected bags, or `None` where the set was not collecting them.
    pub(super) fn bag_pool(&self) -> Option<&BagPool> {
        self.pool.as_ref()
    }

    /// The pool, to put a decomposition in that is not a candidate: one drawn
    /// only to give the recombination a differently shaped tree to read.
    pub(super) fn bag_pool_mut(&mut self) -> Option<&mut BagPool> {
        self.pool.as_mut()
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
        if let Some(pool) = &mut self.pool {
            pool.absorb(&decomposition, origin.stage.slot());
        }
        let (width, total_bag_size) = decomposition.quality_key();
        let mut shape = None;
        self.best_width = Some(self.best_width.map_or(width, |best| best.min(width)));
        // A candidate wider than the incumbent cannot win in either mode, and
        // a plan it will not be sorted on is not worth computing; in
        // all-candidates mode every decomposition is returned and needs one.
        let contender = self.best_quality_key.is_none_or(|best| width <= best.0);
        let mut best = false;
        if !self.retain_only_best || contender {
            // Read off the same decomposition as the width and the total bag
            // size, so the four numbers describe one set of bags. A candidate
            // the set drops is never returned, so nothing ranks it and its
            // shape is not computed.
            shape = self.report_shape.then(|| {
                let (bag_mass, max_separator) = decomposition.shape();
                Shape {
                    bag_mass,
                    max_separator,
                }
            });
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
            shape,
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
    /// order. A decomposition several candidates produced is listed once,
    /// with the origin of the first of them in that order. In best-only mode
    /// this is the winner alone.
    pub(super) fn into_candidates(self) -> Vec<Candidate> {
        let mut retained = self.retained;
        retained.sort_by_key(|retained| retained.quality_key);
        let mut seen = HashSet::with_capacity(retained.len());
        retained
            .into_iter()
            .map(|retained| Candidate {
                decomposition: retained.compaction.apply(retained.decomposition),
                origin: retained.origin,
            })
            .filter(|candidate| seen.insert(bag_tree(&candidate.decomposition)))
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

/// The bags and bag tree of a decomposition in a form that does not depend on
/// the order an algorithm listed its bags or their vertices in: each bag
/// sorted, the bags sorted, and the tree's edges over the sorted positions.
/// Two orders that eliminate the same vertices in a different sequence often
/// build the same bags under the same tree, and this is what says so.
fn bag_tree(decomposition: &TreeDecomposition) -> (Vec<Vec<u32>>, Vec<(usize, usize)>) {
    let mut bags: Vec<(Vec<u32>, usize)> = decomposition
        .bags()
        .iter()
        .enumerate()
        .map(|(index, bag)| {
            let mut vertices = bag.vertices().to_vec();
            vertices.sort_unstable();
            (vertices, index)
        })
        .collect();
    bags.sort_unstable();
    let mut position = vec![0; bags.len()];
    for (sorted, &(_, index)) in bags.iter().enumerate() {
        position[index] = sorted;
    }
    let position = &position;
    let mut edges: Vec<(usize, usize)> = decomposition
        .adjacency()
        .iter()
        .enumerate()
        .flat_map(|(left, neighbours)| {
            neighbours.iter().map(move |&right| {
                let (left, right) = (position[left], position[right]);
                (left.min(right), left.max(right))
            })
        })
        .collect();
    edges.sort_unstable();
    edges.dedup();
    (
        bags.into_iter().map(|(vertices, _)| vertices).collect(),
        edges,
    )
}
