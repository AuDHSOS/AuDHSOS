// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a Diffie-Hellman operation was refused.

use core::fmt;

/// Why a Diffie-Hellman operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DhError {
    /// The prime is not usable as the modulus of a MODP group: even,
    /// zero, with a zero top limb, or wider than the arithmetic allows.
    InvalidPrime,
    /// The generator is below two.
    InvalidGenerator,
    /// A public value is not in the open interval `1 < v < p-1` that
    /// RFC 8268, section 4, requires. A peer's value that fails this is
    /// refused before it is used; a value this side computed fails it
    /// only for a degenerate exponent, and is refused before it is sent.
    PublicValueOutOfRange,
    /// The exchange produced one or `p-1` as the shared secret, which
    /// only a degenerate exponent can do and which no party may use.
    DegenerateSharedSecret,
    /// The output buffer is shorter than a value of this group.
    OutputTooShort,
}

impl fmt::Display for DhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DhError::InvalidPrime => f.write_str("the prime is not a usable MODP modulus"),
            DhError::InvalidGenerator => f.write_str("the generator is below two"),
            DhError::PublicValueOutOfRange => {
                f.write_str("the public value is not between one and the prime less one")
            }
            DhError::DegenerateSharedSecret => {
                f.write_str("the exchange produced a degenerate shared secret")
            }
            DhError::OutputTooShort => {
                f.write_str("the output buffer is too short for a value of this group")
            }
        }
    }
}
