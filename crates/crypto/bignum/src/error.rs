// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a wide-arithmetic operation was refused.

use core::fmt;

/// Why a wide-arithmetic operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BignumError {
    /// The value needs more than [`crate::MAX_LIMBS`] limbs.
    TooWide,
    /// The modulus is zero, or its most significant limb is, which means
    /// the encoding carries leading zero bytes it should not have.
    NotNormalized,
    /// The modulus is even. Montgomery arithmetic needs the low limb to
    /// have an inverse modulo `2^64`, and only an odd limb has one.
    Even,
    /// The base is not below the modulus.
    OutOfRange,
    /// The output buffer is too short for the value the operation
    /// produced.
    OutputTooShort,
}

impl fmt::Display for BignumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BignumError::TooWide => f.write_str("the value is wider than the arithmetic allows"),
            BignumError::NotNormalized => {
                f.write_str("the most significant limb of the modulus is zero")
            }
            BignumError::Even => f.write_str("the modulus is even"),
            BignumError::OutOfRange => f.write_str("the base is not below the modulus"),
            BignumError::OutputTooShort => {
                f.write_str("the output buffer is too short for the result")
            }
        }
    }
}
