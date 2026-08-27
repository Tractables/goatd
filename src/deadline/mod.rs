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
    let soft = budget
        .map(|duration| checked(start, duration, operation))
        .transpose()?;
    let hard = budget
        .map(|duration| checked(start, duration.saturating_mul(2), operation))
        .transpose()?;
    Ok(TwoStage { soft, hard })
}

/// Whether `deadline` has passed.
pub(crate) fn expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|deadline| crate::meter::now() >= deadline)
}

/// How long there is until `deadline` — zero once it has passed.
pub(crate) fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(crate::meter::now())
}

#[cfg(test)]
mod tests;
