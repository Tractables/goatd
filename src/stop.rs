//! A process-wide flag that ends a running solve early.
//!
//! Setting [`stop_flag`] makes every deadline check in the library answer as
//! though the hard deadline had passed, so the construction under way stops and
//! the caller gets the best decomposition found so far. It is read with
//! [`Ordering::Relaxed`], which is what a signal handler can set, and the
//! caller clears it before the next solve.

use std::sync::atomic::{AtomicBool, Ordering};

static STOPPED: AtomicBool = AtomicBool::new(false);

/// The flag that ends a running solve.
///
/// Store `true` to stop; store `false` before starting the next one. A handler
/// that only stores into this flag is async-signal-safe, which is how the
/// command-line tool answers `SIGTERM` with the decomposition it already has.
pub fn stop_flag() -> &'static AtomicBool {
    &STOPPED
}

/// Whether the flag is set.
#[inline]
pub(crate) fn requested() -> bool {
    STOPPED.load(Ordering::Relaxed)
}

/// The address of the flag byte, for the vendored backend's own stopping check.
pub(crate) fn flag_address() -> *const u8 {
    std::ptr::from_ref(&STOPPED).cast::<u8>()
}
