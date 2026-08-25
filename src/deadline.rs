//! The two readings of an absolute deadline every time-bounded search here
//! shares. `None` is the unbounded run: it never expires.

use std::time::{Duration, Instant};

/// Whether `deadline` has passed.
pub(crate) fn expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|d| Instant::now() >= d)
}

/// How long there is until `deadline` — zero once it has passed.
pub(crate) fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}
