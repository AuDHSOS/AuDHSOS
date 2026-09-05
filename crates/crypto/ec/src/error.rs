// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a curve operation was refused.

use core::fmt;

/// Why a curve operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EcError {
    /// The X25519 exchange produced the all-zero value, which means the
    /// peer sent a point of small order. RFC 8446 requires the handshake
    /// to abort.
    ZeroSharedSecret,
    /// An encoded point is not on the curve, or is not a canonical
    /// encoding of a point.
    InvalidPoint,
    /// A scalar is not in the range the algorithm requires.
    InvalidScalar,
    /// The signature does not belong to this message and this key.
    BadSignature,
}

impl fmt::Display for EcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EcError::ZeroSharedSecret => {
                f.write_str("the exchange produced the all-zero shared secret")
            }
            EcError::InvalidPoint => f.write_str("the encoded point is not a point of the curve"),
            EcError::InvalidScalar => f.write_str("the scalar is out of range"),
            EcError::BadSignature => f.write_str("the signature does not verify"),
        }
    }
}
