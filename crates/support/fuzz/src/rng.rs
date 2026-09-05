// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The random numbers a fuzzing run draws.
//!
//! Invariants: a run is reproducible from its seed alone, because nothing
//! here reads a clock or an address; a bound of zero yields zero, so that
//! a caller with an empty range needs no case of its own.

/// The odd 64-bit fraction of the golden ratio, the seed that a seed of
/// zero becomes.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// The multiplier of the `*` in `xorshift64*`.
const MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

/// A `xorshift64*` generator: small and fast, and enough for a mutator,
/// which needs its choices spread out and not unguessable.
#[derive(Clone, Debug)]
pub struct Rng {
    /// The state of the recurrence, never zero.
    state: u64,
}

impl Rng {
    /// A generator seeded with `seed`. Zero is replaced, because zero is a
    /// fixed point of the recurrence and would yield zero forever.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { GOLDEN } else { seed },
        }
    }

    /// The next 64 bits.
    pub const fn next_u64(&mut self) -> u64 {
        let mut state = self.state;
        state ^= state.wrapping_shr(12);
        state ^= state.wrapping_shl(25);
        state ^= state.wrapping_shr(27);
        self.state = state;
        state.wrapping_mul(MULTIPLIER)
    }

    /// A number below `bound`, or zero for a bound of zero.
    ///
    /// The high half of a full-width multiply is uniform over `bound`
    /// without the bias that a remainder of a power-of-two range carries,
    /// and it needs no rejection loop.
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        let drawn = u128::from(self.next_u64());
        let wide = u128::try_from(bound).unwrap_or(u128::MAX);
        usize::try_from(drawn.wrapping_mul(wide).wrapping_shr(64)).unwrap_or(0)
    }

    /// A number in `low ..= high`, or `low` if the range is empty.
    pub fn between(&mut self, low: usize, high: usize) -> usize {
        if high <= low {
            return low;
        }
        let span = high.saturating_sub(low).saturating_add(1);
        low.saturating_add(self.below(span))
    }

    /// One of two outcomes.
    pub const fn coin(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    /// A byte, drawn so that the ends of the range come up more often than
    /// chance alone would have them: a parser reacts to `0x00` and `0xff`
    /// far more than to the bytes between.
    pub fn byte(&mut self) -> u8 {
        let drawn = self.next_u64();
        match drawn & 0b11 {
            0 => 0x00,
            1 => 0xff,
            _ => u8::try_from(drawn.wrapping_shr(8) & 0xff).unwrap_or(0),
        }
    }

    /// Shuffles `slice` in place.
    pub fn shuffle<T>(&mut self, slice: &mut [T]) {
        let mut remaining = slice.len();
        while remaining > 1 {
            let last = remaining.saturating_sub(1);
            slice.swap(last, self.below(remaining));
            remaining = last;
        }
    }
}
