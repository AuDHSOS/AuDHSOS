// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a cursor stopped, or why a text is not an address.

use core::fmt;

/// What this crate found wrong with a buffer, or with the text it was
/// asked to read an address from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WireError {
    /// The cursor was asked for more bytes than it has left. The two
    /// numbers are what was wanted and what remains; the cursor did not
    /// move and nothing was written.
    OutOfBounds {
        /// How many bytes the operation needed.
        needed: usize,
        /// How many bytes the cursor had left.
        available: usize,
    },
    /// The text is not the canonical form of the address it was read as:
    /// a wrong number of groups, a group that is empty, too long, or not a
    /// number, a leading zero, or a byte the form does not allow.
    Address,
    /// A prefix length no IPv4 network has. IPv4 addresses are 32 bits, so
    /// 32 is the last one that names a network.
    PrefixLength(u8),
    /// A length that has to fit in the sixteen bits of a header field and
    /// does not. It reaches the checksum of a transport segment, whose
    /// pseudo-header carries the segment length.
    Length(usize),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::OutOfBounds { needed, available } => {
                write!(f, "{needed} bytes were needed and {available} are left")
            }
            WireError::Address => f.write_str("the text is not an address in canonical form"),
            WireError::PrefixLength(length) => {
                write!(f, "an IPv4 prefix is at most 32 bits, not {length}")
            }
            WireError::Length(length) => {
                write!(f, "{length} bytes do not fit in a sixteen-bit length")
            }
        }
    }
}
