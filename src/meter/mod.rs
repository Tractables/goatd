//! The work meter: a count of the graph work a construction does, and a clock
//! derived from it.
//!
//! Every budgeted search in this crate stops on a decision: the elimination
//! portfolio divides its window between orders, the FlowCutter restart loop
//! stops when its window is gone, the separator search gives up at its cap.
//! Each of those decisions changes which decomposition comes out, and each is
//! normally made against a wall clock — so the decomposition a run produces
//! depends on how fast, and how loaded, the machine was. Two runs of the same
//! graph on the same binary can return different decompositions.
//!
//! This module replaces the quantity those decisions read. It counts *work*:
//! graph elements touched, in one unit shared by every construction here, and
//! serves a clock whose reading is `epoch + spent / rate`. A budget expressed
//! in units is then a budget in work, and the search that spends it makes the
//! same decisions in the same order on every machine. A caller with searches
//! of its own around these charges them to the same meter through [`charge`],
//! so one budget covers the whole construction.
//!
//! # What a unit is
//!
//! One unit is roughly one graph-element touch — a neighbour entry scanned, a
//! pin visited, a bitset word cleared. The absolute size does not matter; what
//! matters is that every construction charges on the *same* scale, so a budget
//! divided between them divides work rather than one backend's private counter.
//!
//! Charges are deliberately pessimistic. A caller that sizes a unit budget
//! from a wall (`milliseconds × UNITS_PER_MS`) should finish inside that wall
//! rather than past it, so where a cost is only known to within a factor the
//! constant sits at the expensive end of the range. The measured price of that
//! choice is a few percent more wall than the same build takes under a
//! wall-clock budget.
//!
//! # The rate is a calibration constant
//!
//! [`UNITS_PER_MS`] converts units to the milliseconds the budgets in this
//! crate are written in. It was fitted by regressing charged units against
//! measured milliseconds over a set of construction runs; it is not a law, and
//! a machine much faster or slower than the one it was fitted on will do more
//! or less real work per unit. That does not affect reproducibility — the same
//! unit budget buys the same *decisions* everywhere — only the wall those
//! decisions take.
//!
//! # How it is armed
//!
//! Nothing here reads the environment. A caller arms the meter for the
//! duration of one construction with [`arm`] and disarms it by dropping the
//! guard. Every deadline the searches in this crate compare against is read
//! from [`now`], so arming the meter moves all of them onto the work clock at
//! once. Off the seam [`charge`] is a predictable branch and [`now`] is
//! `Instant::now()`, so a caller that never arms the meter gets exactly the
//! wall-clock behaviour.
//!
//! The state is thread-local, so a caller running two constructions on two
//! threads meters them independently.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// Work units per millisecond of construction: the calibration constant that
/// converts a unit budget into the milliseconds this crate's budgets are
/// written in, and back.
///
/// Fitted as the median of `units / measured_ms` over per-candidate
/// construction runs. See the module docs on what that does and does not
/// guarantee.
pub const UNITS_PER_MS: u64 = 775_000;

thread_local! {
    /// Where the construction clock was started: the real instant the meter was
    /// armed, paired with the meter reading at that instant. `None` = not
    /// armed, which is every moment outside one metered construction.
    ///
    /// Written only by [`arm`] and by [`Armed::drop`]; read by everything else
    /// here. Being the one flag makes "armed" a single fact rather than
    /// something two cells could disagree about.
    static EPOCH: Cell<Option<(Instant, u64)>> = const { Cell::new(None) };

    /// Units charged on this thread, ever. Monotone and never reset, so a mark
    /// taken at arming turns into elapsed work by plain subtraction.
    static SPENT: Cell<u64> = const { Cell::new(0) };
}

/// The meter is armed for as long as this value lives. Dropping it restores
/// whatever was armed before, so a nested construction cannot leave the meter
/// running for the one that contains it.
#[must_use = "the meter is armed only while the guard is alive"]
pub struct Armed {
    previous: Option<(Instant, u64)>,
}

impl Drop for Armed {
    fn drop(&mut self) {
        EPOCH.with(|c| c.set(self.previous));
    }
}

/// Arm the meter, with the construction clock starting at `now` — the same
/// instant the budget being armed is measured from.
///
/// Pairing the epoch with the current meter reading is what makes a whole
/// construction spend ONE budget: a later construction on the same thread
/// enters with the meter already advanced by the earlier ones, exactly as it
/// would enter with the clock already advanced.
pub fn arm(now: Instant) -> Armed {
    let previous = EPOCH.with(Cell::get);
    EPOCH.with(|c| c.set(Some((now, spent()))));
    Armed { previous }
}

/// Whether the meter is armed, and therefore whether anything charged to it can
/// be read back.
///
/// Hoisted out of hot loops that would otherwise compute a charge nobody
/// records: a charge whose *amount* costs a scan is guarded on this, and one
/// that is a single arithmetic expression is not.
#[inline]
pub fn metering() -> bool {
    EPOCH.with(Cell::get).is_some()
}

/// Charge `units` of construction work. Inert, and unread, when the meter is
/// not armed.
#[inline]
pub fn charge(units: u64) {
    if metering() {
        SPENT.with(|m| m.set(m.get().saturating_add(units)));
    }
}

/// Units charged on this thread so far.
#[inline]
pub fn spent() -> u64 {
    SPENT.with(Cell::get)
}

/// **THE CONSTRUCTION CLOCK.** `Instant::now()` when the meter is not armed;
/// under it, the instant the work says it is — the epoch plus the work charged
/// since, converted at [`UNITS_PER_MS`].
///
/// Every budgeted DECISION in this crate reads this instead of the real clock:
/// the portfolio's slot and refinement deadlines, the elimination core's soft
/// and hard deadlines, the separator search's cap, the wall a FlowCutter
/// build converts into its work budget.
///
/// It is an `Instant` and not a duration or a counter because that is the shape
/// the budgets already have: a deadline armed once travels as a bare
/// `Instant` through the portfolio into the elimination core and is compared
/// against the clock in a dozen places that never see the site that set it.
/// Converting the clock those comparisons read converts all of them at once
/// and leaves every deadline expression untouched.
///
/// Monotone, because the meter is: it never runs backwards and never precedes
/// the epoch. A run that charged enough units to overflow the addition
/// saturates at the epoch rather than panicking.
pub fn now() -> Instant {
    match EPOCH.with(Cell::get) {
        None => Instant::now(),
        Some((epoch, mark)) => {
            let ms = spent().saturating_sub(mark) / UNITS_PER_MS;
            epoch
                .checked_add(Duration::from_millis(ms))
                .unwrap_or(epoch)
        }
    }
}

/// A unit count as the milliseconds it converts to, for handing a budget in
/// units to code written in milliseconds.
pub fn wall_ms_for_units(units: u64) -> u64 {
    units / UNITS_PER_MS
}

#[cfg(test)]
mod tests;
