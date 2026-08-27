//! The xorshift64 generator every seeded search here draws from.

/// Deterministic xorshift64 with the usual 13 / 7 / 17 shift triple.
///
/// State 0 is the recurrence's fixed point and yields an endless run of zeros,
/// so a caller starting from a caller-supplied seed has to move it off zero
/// first — `SEED_OFFSET` is one of the two offsets in use for that.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Xorshift64(u64);

/// Odd constant one group of callers adds to its seed before starting a stream
/// to move seed 0 off the fixed point and send nearby seeds to unrelated
/// streams. The golden ratio scaled to 2^64, the same constant SplitMix64 steps
/// its state by.
///
/// The bisectors add 1 instead, through [`bisector_stream`] — changing either
/// offset changes the stream it feeds.
pub(crate) const SEED_OFFSET: u64 = 0x9E37_79B9_7F4A_7C15;

/// Starts a bisector's stream: `+ 1` keeps a seed of 0 off the recurrence's
/// zero fixed point. This offset is part of the stream every existing seed has
/// always drawn from — changing it reshuffles all of them.
pub(crate) fn bisector_stream(seed: u64) -> Xorshift64 {
    Xorshift64::from_state(seed.wrapping_add(1))
}

/// Derives the stream seed for one partitioner restart. The finalizer keeps
/// nearby caller-supplied seeds from producing nearby streams.
pub(crate) fn restart_seed(base_seed: u64, restart: usize) -> u64 {
    let restart_stream = (restart as u64).wrapping_mul(7919).wrapping_add(42);
    restart_stream ^ fmix64(base_seed)
}

/// SplitMix64 finalizer.
fn fmix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

impl Xorshift64 {
    /// Takes `state` exactly as given; callers apply any off-zero offset
    /// themselves before calling.
    pub(crate) fn from_state(state: u64) -> Self {
        Xorshift64(state)
    }

    /// The next 64 bits of the stream.
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// The low 32 bits of [`Self::next_u64`].
    pub(crate) fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }
}

#[cfg(test)]
mod tests;
