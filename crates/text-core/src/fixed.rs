// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Signed Q32.32 arithmetic, rounded to nearest with ties to even.

use crate::FontError;

/// A deterministic signed Q32.32 number. Overflow is an error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Fixed(i64);

impl Fixed {
    /// Additive identity.
    pub const ZERO: Self = Self(0);
    /// One unit.
    pub const ONE: Self = Self(1_i64 << 32);

    /// Interpret an exact Q32.32 bit pattern.
    #[must_use]
    pub const fn from_bits(bits: i64) -> Self {
        Self(bits)
    }

    /// Return the exact Q32.32 bit pattern.
    #[must_use]
    pub const fn bits(self) -> i64 {
        self.0
    }

    /// Convert an integer exactly.
    #[must_use]
    pub fn from_i32(value: i32) -> Self {
        Self(i64::from(value) << 32)
    }

    /// Convert a signed 16.16 bit pattern exactly.
    #[must_use]
    pub fn from_16_16(value: i32) -> Self {
        Self(i64::from(value) << 16)
    }

    /// Convert a signed 2.14 bit pattern exactly.
    #[must_use]
    pub fn from_2_14(value: i16) -> Self {
        Self(i64::from(value) << 18)
    }

    /// Add two numbers.
    /// # Errors
    /// Returns `Overflow` when the result exceeds Q32.32.
    pub fn checked_add(self, rhs: Self) -> Result<Self, FontError> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(FontError::Overflow)
    }

    /// Subtract two numbers.
    /// # Errors
    /// Returns `Overflow` when the result exceeds Q32.32.
    pub fn checked_sub(self, rhs: Self) -> Result<Self, FontError> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(FontError::Overflow)
    }

    /// Negate a number.
    /// # Errors
    /// Returns `Overflow` for the minimum bit pattern.
    pub fn checked_neg(self) -> Result<Self, FontError> {
        self.0.checked_neg().map(Self).ok_or(FontError::Overflow)
    }

    /// Multiply and round once.
    /// # Errors
    /// Returns `Overflow` when the result exceeds Q32.32.
    pub fn checked_mul(self, rhs: Self) -> Result<Self, FontError> {
        Self::rounded(product(self.0, rhs.0), i128::from(Self::ONE.0))
    }

    /// Divide and round once.
    /// # Errors
    /// Returns `InvalidTable` for zero, or `Overflow` for an unrepresentable result.
    pub fn checked_div(self, rhs: Self) -> Result<Self, FontError> {
        Self::rounded(product(self.0, Self::ONE.0), i128::from(rhs.0))
    }

    /// Scale by an integer ratio with one rounding and a wide intermediate.
    /// # Errors
    /// Returns `InvalidTable` for zero denominator, or `Overflow` for the result.
    pub fn mul_ratio(self, numerator: i64, denominator: i64) -> Result<Self, FontError> {
        Self::rounded(product(self.0, numerator), i128::from(denominator))
    }

    /// Multiply two fixed-point values and divide by an integer, rounding once.
    /// # Errors
    /// Returns `InvalidTable` for zero divisor, or `Overflow` for the result.
    pub fn mul_div(self, rhs: Self, divisor: i64) -> Result<Self, FontError> {
        Self::rounded(product(self.0, rhs.0), product(divisor, Self::ONE.0))
    }

    fn rounded(n: i128, d: i128) -> Result<Self, FontError> {
        let q = n.checked_div(d).ok_or(FontError::InvalidTable)?;
        let r = n.checked_rem(d).ok_or(FontError::InvalidTable)?;
        let twice = r.unsigned_abs().checked_mul(2).ok_or(FontError::Overflow)?;
        let round = twice > d.unsigned_abs() || (twice == d.unsigned_abs() && q & 1 != 0);
        let q = if round {
            q.checked_add(if (n < 0) == (d < 0) { 1 } else { -1 })
                .ok_or(FontError::Overflow)?
        } else {
            q
        };
        i64::try_from(q).map(Self).map_err(|_| FontError::Overflow)
    }
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "two i64 operands fit an i128 product"
)]
fn product(a: i64, b: i64) -> i128 {
    i128::from(a) * i128::from(b)
}
