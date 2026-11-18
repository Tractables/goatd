//! A wall-clock cutoff for every search on a thread, held by [`WallGuard`].
//!
//! Inside the crate, this module also builds and reads the deadlines the
//! searches pace themselves with. `None` is an unbounded run and never expires.

use std::cell::Cell;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use crate::Error;

/// The soft and hard cutoffs used by elimination-based searches.
pub(crate) struct TwoStage {
    pub(crate) soft: Option<Instant>,
    pub(crate) hard: Option<Instant>,
}

/// Add `budget` to `start`, with a consistent input error on overflow.
pub(crate) fn checked(start: Instant, budget: Duration, operation: &str) -> Result<Instant, Error> {
    start
        .checked_add(budget)
        .ok_or_else(|| Error::InvalidInput(format!("{operation} budget is too large")))
}

/// Build soft and hard cutoffs, using twice the soft budget when no separate
/// hard budget is given.
pub(crate) fn staged(
    start: Instant,
    soft_budget: Option<Duration>,
    hard_budget: Option<Duration>,
    operation: &str,
) -> Result<TwoStage, Error> {
    let effective_hard = match (soft_budget, hard_budget) {
        (None, None) => None,
        (None, Some(_)) => {
            return Err(Error::InvalidInput(format!(
                "{operation} hard budget needs a soft budget"
            )));
        }
        (Some(soft), Some(hard)) if hard < soft => {
            return Err(Error::InvalidInput(format!(
                "{operation} hard budget must be at least its soft budget"
            )));
        }
        (Some(_), Some(hard)) => Some(hard),
        (Some(soft), None) => Some(soft.saturating_mul(2)),
    };
    let soft = soft_budget
        .map(|duration| checked(start, duration, operation))
        .transpose()?;
    let hard = effective_hard
        .map(|duration| checked(start, duration, operation))
        .transpose()?;
    Ok(TwoStage { soft, hard })
}

/// Whether `deadline` has passed, or the caller asked the solve to stop.
///
/// A set stop flag answers here exactly as an expired deadline does, including
/// on a run that was given no deadline at all.
pub(crate) fn expired(deadline: Option<Instant>) -> bool {
    crate::stop::requested()
        || wall_cutoff().is_some_and(|end| Instant::now() >= end)
        || deadline.is_some_and(|deadline| crate::meter::now() >= deadline)
}

thread_local! {
    static WALL_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// A wall-clock cutoff for every search on this thread while the guard is held.
///
/// Once the instant passes, each search returns the best decomposition it
/// holds, as it does at its own deadline; the vendored FlowCutter still
/// finishes its first decomposition. The cutoff holds whether or not
/// [`crate::meter::arm`] has put the run's budgets on the work clock, so a
/// metered run is repeatable up to the cutoff and bounded in wall time after
/// it.
///
/// Guards nest: an inner guard cannot extend the cutoff of an enclosing one,
/// and dropping a guard restores the cutoff in force before it. `None` adds no
/// cutoff. The guard belongs to the thread that made it.
#[must_use = "the cutoff lasts as long as the guard is held"]
pub struct WallGuard {
    previous: Option<Instant>,
    _thread: PhantomData<*const ()>,
}

impl WallGuard {
    /// Stop searches on this thread at `deadline`, or at the cutoff already in
    /// force if that is earlier.
    pub fn new(deadline: Option<Instant>) -> Self {
        let previous = WALL_DEADLINE.with(|limit| {
            let previous = limit.get();
            limit.set(match (previous, deadline) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            });
            previous
        });
        Self {
            previous,
            _thread: PhantomData,
        }
    }
}

impl Drop for WallGuard {
    fn drop(&mut self) {
        WALL_DEADLINE.with(|limit| limit.set(self.previous));
    }
}

/// The wall-clock cutoff in force on this thread: the earliest instant of the
/// [`WallGuard`]s held, or `None` without one.
#[must_use]
pub fn wall_cutoff() -> Option<Instant> {
    WALL_DEADLINE.with(Cell::get)
}

/// How long there is until `deadline` — zero once it has passed, and zero
/// whenever [`expired`] answers true for one of its other reasons: the caller
/// asked the solve to stop, or an enclosing wall cutoff has run out. Callers
/// size a sub-budget from this and then pace it with `expired`, so the two
/// have to agree about whether there is time left.
pub(crate) fn remaining(deadline: Instant) -> Duration {
    if expired(None) {
        return Duration::ZERO;
    }
    deadline.saturating_duration_since(crate::meter::now())
}

#[cfg(test)]
mod tests;
