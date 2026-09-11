//! Cache hints for the walks that read a large table at scattered indices.
//!
//! A walk whose next index comes out of the data it is walking cannot be
//! predicted by the hardware, so each touch waits for its own miss. Asking for
//! the line a fixed number of entries ahead hides that latency. The hint is
//! free of arithmetic: it never reads the value, so a walk that prefetches
//! computes exactly what the same walk without it computes.

/// Ask the cache for `slice[index]` ahead of a walk that will read it.
///
/// `index` is not read here, so an index past the end is harmless. A no-op off
/// x86-64.
#[inline(always)]
pub(crate) fn prefetch<T>(slice: &[T], index: usize) {
    #[cfg(target_arch = "x86_64")]
    if index < slice.len() {
        // SAFETY: prefetching never faults and the address is inside the
        // slice; the intrinsic is available on every x86-64.
        unsafe {
            std::arch::x86_64::_mm_prefetch(
                slice.as_ptr().add(index).cast::<i8>(),
                std::arch::x86_64::_MM_HINT_T0,
            );
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (slice, index);
    }
}

/// Vertices from which a walk prefetches. Below this the table it reads at
/// scattered indices is small enough to stay in cache, so the walk waits on
/// nothing and a prefetch only costs it an instruction per entry; at this
/// size and above the scattered read is the miss the walk waits on.
pub(crate) const PREFETCH_MIN_VERTICES: usize = 1 << 16;

/// Whether a walk over a table of `len` entries prefetches.
#[inline(always)]
pub(crate) fn prefetching(len: usize) -> bool {
    len >= PREFETCH_MIN_VERTICES
}
