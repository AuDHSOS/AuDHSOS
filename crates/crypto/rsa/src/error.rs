// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why an RSA operation was refused.

use core::fmt;

/// Why an RSA operation was refused.
///
/// Verification has one answer for every way it can fail. A signature
/// that is the wrong length, one that is not below the modulus, one whose
/// padding is short, and one whose digest does not match are all the same
/// refusal, because telling them apart tells whoever sent the signature
/// which of them it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RsaError {
    /// The modulus is not the shape an RSA modulus has: even, wider than
    /// the arithmetic allows, or without its top bit set.
    InvalidModulus,
    /// The public exponent is even, below three, or too wide to be one.
    InvalidExponent,
    /// The signature does not belong to this message and this key.
    BadSignature,
}

impl fmt::Display for RsaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RsaError::InvalidModulus => f.write_str("the modulus is not a usable RSA modulus"),
            RsaError::InvalidExponent => f.write_str("the public exponent is out of range"),
            RsaError::BadSignature => f.write_str("the signature does not verify"),
        }
    }
}
