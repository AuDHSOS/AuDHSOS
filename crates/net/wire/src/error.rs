// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a cursor stopped, or why a text is not an address.

use core::fmt;

use crate::addr::{IpVersion, Ipv4Cidr, Ipv6Cidr};

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
    /// A prefix longer than the address of its IP version.
    PrefixLength {
        /// The address version.
        version: IpVersion,
        /// The rejected prefix length.
        length: u8,
    },
    /// A length that has to fit in a header field and does not: sixteen
    /// bits for an IPv4 pseudo-header, thirty-two for an IPv6 one.
    Length(usize),
    /// One address of each family where both had to be of one. There is no
    /// IPv4-mapped form here, so a v4 source with a v6 destination is an
    /// error and never a conversion (D-69).
    MixedFamilies,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::OutOfBounds { needed, available } => {
                write!(f, "{needed} bytes were needed and {available} are left")
            }
            WireError::Address => f.write_str("the text is not an address in canonical form"),
            WireError::PrefixLength { version, length } => {
                let limit = match version {
                    IpVersion::V4 => Ipv4Cidr::MAX_PREFIX_LEN,
                    IpVersion::V6 => Ipv6Cidr::MAX_PREFIX_LEN,
                };
                write!(
                    f,
                    "an {version} prefix is at most {limit} bits, not {length}"
                )
            }
            WireError::Length(length) => {
                write!(f, "{length} bytes do not fit in the length field")
            }
            WireError::MixedFamilies => f.write_str("the two addresses are of different families"),
        }
    }
}
