// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a key derivation was refused.

use core::fmt;

/// Why a key derivation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HashError {
    /// More output was asked of HKDF than the construction can produce:
    /// RFC 5869 bounds it at 255 times the hash length.
    OutputTooLong,
}

impl fmt::Display for HashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HashError::OutputTooLong => {
                f.write_str("HKDF cannot produce more than 255 hash lengths of output")
            }
        }
    }
}
