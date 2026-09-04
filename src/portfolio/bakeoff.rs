//! Which family of candidates gets the rest of the window.
//!
//! The portfolio spreads its budget over a fixed head, a diverse pass, a
//! series of weighted stages, min-fill restarts and a trailing FlowCutter
//! candidate, and every graph gets the same split. htd does the opposite: it
//! runs a few rounds of its three ordering rules, keeps the one that did best
//! and gives it everything that is left. This is that rule over goatd's
//! families.
//!
//! Only the scoring lives here. Running the candidates is
//! [`super::run_portfolio`], which owns the graph and the clock.

use super::trace::BakeoffArm;

/// A family whose best is more than this many halves of the round leader's
/// best is out of the running. htd's own figure, and the shape of it matters
/// more than the value: an arm three widths behind on a graph of width two is
/// not going to catch up in the window that is left.
const DROP_NUMERATOR: u64 = 3;
const DROP_DENOMINATOR: u64 = 2;

/// Rounds the bake-off has to complete before it will commit. One round is a
/// single draw per family, and a single draw says more about the seed than
/// about the family.
pub(super) const MIN_ROUNDS: u32 = 2;

/// One family's record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ArmRecord {
    arm: BakeoffArm,
    /// Rounds in which this family came back with a decomposition.
    scored: u32,
    /// Its widths over those rounds.
    summed_width: u64,
    /// The narrowest of them.
    best_width: Option<u32>,
    /// Still in the running.
    alive: bool,
}

impl ArmRecord {
    fn new(arm: BakeoffArm) -> Self {
        Self {
            arm,
            scored: 0,
            summed_width: 0,
            best_width: None,
            alive: true,
        }
    }
}

/// The rounds, the families still in them, and what each has produced.
#[derive(Clone, Debug)]
pub(super) struct Bakeoff {
    arms: Vec<ArmRecord>,
    rounds: u32,
}

impl Bakeoff {
    pub(super) fn new(arms: &[BakeoffArm]) -> Self {
        Self {
            arms: arms.iter().copied().map(ArmRecord::new).collect(),
            rounds: 0,
        }
    }

    /// The families still in the running, in the order they were given.
    pub(super) fn alive(&self) -> Vec<BakeoffArm> {
        self.arms
            .iter()
            .filter(|record| record.alive)
            .map(|record| record.arm)
            .collect()
    }

    /// Rounds completed.
    pub(super) fn rounds(&self) -> u32 {
        self.rounds
    }

    /// Record the width `arm` reached this round. A family that came back with
    /// nothing is simply not recorded, and scores no round.
    pub(super) fn record(&mut self, arm: BakeoffArm, width: u32) {
        let Some(record) = self.arms.iter_mut().find(|record| record.arm == arm) else {
            return;
        };
        record.scored += 1;
        record.summed_width = record.summed_width.saturating_add(u64::from(width));
        record.best_width = Some(match record.best_width {
            Some(best) => best.min(width),
            None => width,
        });
    }

    /// Close a round: drop the families that are behind, and count the round.
    ///
    /// Behind means one and a half times the round leader's narrowest, or
    /// nothing at all after a round that somebody answered. The last family
    /// standing is never dropped, so there is always something to commit to.
    pub(super) fn end_round(&mut self) {
        self.rounds += 1;
        let leader = self
            .arms
            .iter()
            .filter(|record| record.alive)
            .filter_map(|record| record.best_width)
            .min();
        let Some(leader) = leader else { return };
        let bound = u64::from(leader)
            .saturating_mul(DROP_NUMERATOR)
            .div_ceil(DROP_DENOMINATOR);
        for record in &mut self.arms {
            if !record.alive {
                continue;
            }
            record.alive = record
                .best_width
                .is_some_and(|best| u64::from(best) <= bound);
        }
    }

    /// The family the rest of the window goes to, and its narrowest round.
    ///
    /// Among the families still in the running: smallest mean width over the
    /// rounds each answered, then the narrowest single round, then the order
    /// the families were given. Mean rather than htd's sum because a family can
    /// miss a round here — a FlowCutter slice can come back with nothing — and
    /// a smaller sum would then reward it for that.
    pub(super) fn winner(&self) -> Option<(BakeoffArm, u32)> {
        if self.rounds < MIN_ROUNDS {
            return None;
        }
        let mut best: Option<&ArmRecord> = None;
        for record in self
            .arms
            .iter()
            .filter(|record| record.alive && record.scored > 0)
        {
            let better = match best {
                None => true,
                // a/b < c/d without dividing.
                Some(current) => {
                    let left = record.summed_width * u64::from(current.scored);
                    let right = current.summed_width * u64::from(record.scored);
                    left < right || (left == right && record.best_width < current.best_width)
                }
            };
            if better {
                best = Some(record);
            }
        }
        best.map(|record| {
            (
                record.arm,
                record
                    .best_width
                    .expect("a family that scored a round has a width"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arms() -> Vec<BakeoffArm> {
        vec![
            BakeoffArm::MinFill,
            BakeoffArm::MinDegree,
            BakeoffArm::FlowCutter,
        ]
    }

    #[test]
    fn one_round_does_not_commit() {
        let mut bakeoff = Bakeoff::new(&arms());
        bakeoff.record(BakeoffArm::MinFill, 10);
        bakeoff.end_round();
        assert_eq!(bakeoff.rounds(), 1);
        assert_eq!(bakeoff.winner(), None);
    }

    #[test]
    fn smallest_mean_width_wins() {
        let mut bakeoff = Bakeoff::new(&arms());
        for (fill, degree) in [(10, 11), (12, 11)] {
            bakeoff.record(BakeoffArm::MinFill, fill);
            bakeoff.record(BakeoffArm::MinDegree, degree);
            bakeoff.end_round();
        }
        // Means are 11 each; min-fill's narrowest round is 10.
        assert_eq!(bakeoff.winner(), Some((BakeoffArm::MinFill, 10)));
        let mut bakeoff = Bakeoff::new(&arms());
        for (fill, degree) in [(10, 9), (12, 9)] {
            bakeoff.record(BakeoffArm::MinFill, fill);
            bakeoff.record(BakeoffArm::MinDegree, degree);
            bakeoff.end_round();
        }
        assert_eq!(bakeoff.winner(), Some((BakeoffArm::MinDegree, 9)));
    }

    #[test]
    fn a_missed_round_neither_helps_nor_disqualifies() {
        let mut bakeoff = Bakeoff::new(&arms());
        // min-degree answers once at width 10 and misses the second round;
        // min-fill answers both at 11. Summed width would put min-degree ahead
        // for having missed one, and its mean puts it ahead on its merits.
        bakeoff.record(BakeoffArm::MinFill, 11);
        bakeoff.record(BakeoffArm::MinDegree, 10);
        bakeoff.end_round();
        bakeoff.record(BakeoffArm::MinFill, 11);
        bakeoff.end_round();
        assert_eq!(bakeoff.winner(), Some((BakeoffArm::MinDegree, 10)));
        assert!(bakeoff.alive().contains(&BakeoffArm::MinFill));
    }

    #[test]
    fn a_dropped_family_cannot_win() {
        let mut bakeoff = Bakeoff::new(&arms());
        // FlowCutter's one round is the narrowest anybody reached, but it is
        // over the bound in the next round and out of the running.
        bakeoff.record(BakeoffArm::MinFill, 10);
        bakeoff.record(BakeoffArm::FlowCutter, 9);
        bakeoff.end_round();
        bakeoff.record(BakeoffArm::MinFill, 5);
        bakeoff.end_round();
        assert_eq!(bakeoff.alive(), vec![BakeoffArm::MinFill]);
        assert_eq!(bakeoff.winner(), Some((BakeoffArm::MinFill, 5)));
    }

    #[test]
    fn far_behind_is_dropped_and_silent_is_dropped() {
        let mut bakeoff = Bakeoff::new(&arms());
        bakeoff.record(BakeoffArm::MinFill, 10);
        bakeoff.record(BakeoffArm::MinDegree, 15);
        bakeoff.record(BakeoffArm::FlowCutter, 16);
        bakeoff.end_round();
        // 15 is exactly the bound, 16 is over it, and nothing is silent.
        assert_eq!(
            bakeoff.alive(),
            vec![BakeoffArm::MinFill, BakeoffArm::MinDegree]
        );
        bakeoff.record(BakeoffArm::MinFill, 10);
        bakeoff.end_round();
        // min-degree answered nothing this round but its earlier best stands.
        assert_eq!(
            bakeoff.alive(),
            vec![BakeoffArm::MinFill, BakeoffArm::MinDegree]
        );
    }

    #[test]
    fn a_family_that_never_answers_is_dropped() {
        let mut bakeoff = Bakeoff::new(&arms());
        bakeoff.record(BakeoffArm::MinFill, 10);
        bakeoff.end_round();
        assert_eq!(bakeoff.alive(), vec![BakeoffArm::MinFill]);
    }

    #[test]
    fn a_round_nobody_answers_drops_nobody() {
        let mut bakeoff = Bakeoff::new(&arms());
        bakeoff.end_round();
        assert_eq!(bakeoff.alive().len(), 3);
        assert_eq!(bakeoff.winner(), None);
    }
}
