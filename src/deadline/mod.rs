//! Shared construction and inspection of absolute deadlines. `None` is an
//! unbounded run and never expires.

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

/// Build the soft cutoff at `budget` and the hard cutoff at twice `budget`.
pub(crate) fn two_stage(
    start: Instant,
    budget: Option<Duration>,
    operation: &str,
) -> Result<TwoStage, Error> {
    staged(start, budget, None, operation)
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
    crate::stop::requested() || deadline.is_some_and(|deadline| crate::meter::now() >= deadline)
}

/// How long there is until `deadline` — zero once it has passed, and zero once
/// the caller has asked the solve to stop.
pub(crate) fn remaining(deadline: Instant) -> Duration {
    if crate::stop::requested() {
        return Duration::ZERO;
    }
    deadline.saturating_duration_since(crate::meter::now())
}

#[cfg(test)]
mod tests;
