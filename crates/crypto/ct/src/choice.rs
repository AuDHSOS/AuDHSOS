// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The branchless boolean.
//!
//! Invariant: the wrapped byte is `0` or `1` and nothing else. Every
//! constructor establishes it, every operation preserves it, and the masks
//! derived from it are therefore `0x00` or all ones.

use core::ops::{BitAnd, BitOr, BitXor, Not};

/// The result of comparing values that must not be branched on.
///
/// A `Choice` is produced by a constant-time comparison and consumed by a
/// constant-time selection. Turning it into a `bool` with
/// [`Choice::is_true`] is the point where the result becomes a decision the
/// program acts on, and therefore observable; that is allowed exactly where
/// the decision is public anyway, such as accepting or rejecting a
/// record whose tag failed to verify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice(u8);

impl Choice {
    /// The true choice.
    pub const YES: Choice = Choice(1);
    /// The false choice.
    pub const NO: Choice = Choice(0);

    /// The choice carried by the lowest bit of `value`. Every other bit is
    /// discarded, so any accumulator can be turned into a choice.
    #[must_use]
    pub const fn from_lsb(value: u8) -> Choice {
        Choice(value & 1)
    }

    /// The choice that is true exactly when `value` is zero.
    ///
    /// The standard trick: for every non-zero byte, either the value or its
    /// two's complement has the high bit set.
    #[must_use]
    pub const fn is_zero_u8(value: u8) -> Choice {
        let folded = value | value.wrapping_neg();
        Choice(folded.wrapping_shr(7) ^ 1)
    }

    /// The choice that is true exactly when `value` is zero.
    ///
    /// The same trick as [`Choice::is_zero_u8`], one word wide: for every
    /// non-zero word, either the value or its two's complement has the
    /// high bit set.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "the shift leaves one bit, so the narrowing cast keeps the whole value"
    )]
    pub const fn is_zero_u64(value: u64) -> Choice {
        let folded = value | value.wrapping_neg();
        Choice((folded.wrapping_shr(63) as u8) ^ 1)
    }

    /// The wrapped byte, `0` or `1`.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }

    /// `0xFF` for the true choice, `0x00` for the false one.
    #[must_use]
    pub const fn mask_u8(self) -> u8 {
        0u8.wrapping_sub(self.0)
    }

    /// All ones for the true choice, zero for the false one.
    #[must_use]
    #[expect(clippy::as_conversions, reason = "widening cast in a const fn")]
    pub const fn mask_u32(self) -> u32 {
        0u32.wrapping_sub(self.0 as u32)
    }

    /// All ones for the true choice, zero for the false one.
    #[must_use]
    #[expect(clippy::as_conversions, reason = "widening cast in a const fn")]
    pub const fn mask_u64(self) -> u64 {
        0u64.wrapping_sub(self.0 as u64)
    }

    /// The choice as a `bool`.
    ///
    /// This is the deliberate exit from constant time: the caller branches
    /// on the result. Call it only where the outcome is public, and never
    /// on a value derived from key material by any path other than a
    /// comparison whose result the protocol reveals anyway.
    #[must_use]
    pub const fn is_true(self) -> bool {
        self.0 == 1
    }
}

impl From<bool> for Choice {
    fn from(value: bool) -> Choice {
        Choice(u8::from(value))
    }
}

impl Not for Choice {
    type Output = Choice;

    fn not(self) -> Choice {
        Choice(self.0 ^ 1)
    }
}

impl BitAnd for Choice {
    type Output = Choice;

    fn bitand(self, other: Choice) -> Choice {
        Choice(self.0 & other.0)
    }
}

impl BitOr for Choice {
    type Output = Choice;

    fn bitor(self, other: Choice) -> Choice {
        Choice(self.0 | other.0)
    }
}

impl BitXor for Choice {
    type Output = Choice;

    fn bitxor(self, other: Choice) -> Choice {
        Choice(self.0 ^ other.0)
    }
}
