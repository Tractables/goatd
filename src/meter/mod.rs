//! A work-based clock for repeatable budget decisions.
//!
//! Normally [`now`] is the wall clock. While the guard returned by [`arm`] is
//! alive it advances by charged graph work instead, so a fixed budget stops
//! after the same work for a given graph and seed, independent of machine
//! load. [`charge`] counts either way: the library also uses the count to
//! decide how often a loop should read the clock.
//!
//! The meter is thread-local and may be nested. Callers that perform graph work
//! around goatd can charge it to the same budget with [`charge`].

use std::cell::Cell;
use std::time::{Duration, Instant};

/// Calibration used to express work budgets through duration-based APIs.
pub const UNITS_PER_MS: u64 = 775_000;

thread_local! {
    /// Clock origin and the charged-unit count at that point.
    static EPOCH: Cell<Option<(Instant, u64)>> = const { Cell::new(None) };

    /// Monotone units charged on this thread.
    static SPENT: Cell<u64> = const { Cell::new(0) };
}

/// The meter is armed for as long as this value lives. Dropping it restores
/// whatever was armed before, so a nested construction cannot leave the meter
/// running for the one that contains it.
#[must_use = "the meter is armed only while the guard is alive"]
pub struct Guard {
    previous: Option<(Instant, u64)>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        EPOCH.with(|c| c.set(self.previous));
    }
}

/// Use charged work as the clock until the returned guard is dropped.
///
/// `epoch` should be the instant from which the caller measures its budget.
pub fn arm(epoch: Instant) -> Guard {
    let previous = EPOCH.with(Cell::get);
    EPOCH.with(|c| c.set(Some((epoch, units_spent()))));
    Guard { previous }
}

/// Whether charged work currently drives [`now`].
#[inline]
pub fn is_armed() -> bool {
    EPOCH.with(Cell::get).is_some()
}

/// Charge construction work.
///
/// Counted whether or not the meter is armed: [`now`] reads the count only
/// while armed, but the library paces its deadline reads by it either way, so
/// a loop whose iterations differ in cost by orders of magnitude can ask for
/// the clock on the work it has done rather than on the iterations it has run.
#[inline]
pub fn charge(units: u64) {
    SPENT.with(|m| m.set(m.get().saturating_add(units)));
}

/// Units charged on this thread so far.
#[inline]
pub fn units_spent() -> u64 {
    SPENT.with(Cell::get)
}

/// The wall clock when unarmed; otherwise `epoch + charged work`.
pub fn now() -> Instant {
    match EPOCH.with(Cell::get) {
        None => Instant::now(),
        Some((epoch, mark)) => {
            let ms = units_spent().saturating_sub(mark) / UNITS_PER_MS;
            saturating_add_milliseconds(epoch, ms)
        }
    }
}

/// Add as much of `milliseconds` as this platform's [`Instant`] range can
/// represent. The binary search runs only on the overflow path.
fn saturating_add_milliseconds(epoch: Instant, milliseconds: u64) -> Instant {
    if let Some(result) = epoch.checked_add(Duration::from_millis(milliseconds)) {
        return result;
    }

    let mut representable = 0;
    let mut too_large = milliseconds;
    while representable < too_large {
        let candidate = representable + (too_large - representable).div_ceil(2);
        if epoch
            .checked_add(Duration::from_millis(candidate))
            .is_some()
        {
            representable = candidate;
        } else {
            too_large = candidate - 1;
        }
    }
    epoch
        .checked_add(Duration::from_millis(representable))
        .expect("adding zero milliseconds to an Instant is representable")
}

/// A unit count as the milliseconds it converts to, for handing a budget in
/// units to code written in milliseconds.
pub fn milliseconds_for_units(units: u64) -> u64 {
    units / UNITS_PER_MS
}
