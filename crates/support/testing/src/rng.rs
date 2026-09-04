// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A small deterministic pseudo-random generator (`SplitMix64`).
//!
//! Invariants: the same seed yields the same sequence; every bounded draw
//! is uniform over its range.

/// A deterministic pseudo-random generator.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// A generator that produces the sequence determined by `seed`.
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        Rng { state: seed }
    }

    /// The next 64 random bits.
    pub const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform value in `0..bound`. Returns `0` for `bound == 0`.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        // Rejection sampling: draws above `zone` would bias the remainder.
        let zone = u64::MAX.wrapping_sub(u64::MAX.checked_rem(bound).unwrap_or(0));
        loop {
            let draw = self.next_u64();
            if draw < zone {
                return draw.checked_rem(bound).unwrap_or(0);
            }
        }
    }

    /// A uniform value in `low..=high`. The bounds may be given in any
    /// order.
    pub fn range(&mut self, low: u64, high: u64) -> u64 {
        let (low, high) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        match high.wrapping_sub(low).checked_add(1) {
            Some(span) => low.wrapping_add(self.below(span)),
            None => self.next_u64(),
        }
    }

    /// A uniformly random boolean.
    pub const fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    /// `true` with probability `numerator / denominator`.
    pub fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        self.below(denominator) < numerator
    }
}
