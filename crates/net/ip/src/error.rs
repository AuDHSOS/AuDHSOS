// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a datagram could not be read, written, or delivered.

use core::fmt;

use net_wire::WireError;

/// What this crate found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IpError {
    /// A cursor ran out of buffer.
    Wire(WireError),
    /// The version field is not four.
    Version(u8),
    /// The header length field is below the five words a header takes, or
    /// beyond the bytes there are.
    HeaderLength(u8),
    /// The total length field is below the header or beyond the frame.
    TotalLength(u16),
    /// The header checksum does not verify. The value is the one the
    /// datagram carried.
    Checksum(u16),
    /// The time to live reached zero.
    TimeExceeded,
    /// A datagram longer than the interface MTU carries the don't-fragment
    /// bit, so it can be neither sent nor cut up.
    WouldFragment {
        /// How long the datagram is.
        length: usize,
        /// What the interface carries.
        mtu: usize,
    },
    /// No route reaches the destination.
    NoRoute,
    /// Two fragments of one datagram claim the same bytes and disagree, so
    /// the whole datagram is discarded. RFC 791 leaves the case open and
    /// an overlap is how a filter is walked past.
    OverlappingFragment,
    /// The datagram would not fit the reassembly buffer.
    TooLarge(usize),
    /// The message is shorter than the ICMP header, or its checksum does
    /// not verify.
    BadIcmp,
}

impl From<WireError> for IpError {
    fn from(error: WireError) -> IpError {
        IpError::Wire(error)
    }
}

impl fmt::Display for IpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpError::Wire(error) => error.fmt(f),
            IpError::Version(version) => write!(f, "the version is {version} and not four"),
            IpError::HeaderLength(words) => {
                write!(f, "a header of {words} words is not one this system reads")
            }
            IpError::TotalLength(length) => {
                write!(f, "a total length of {length} is not one this datagram has")
            }
            IpError::Checksum(carried) => {
                write!(f, "the header checksum {carried:#06x} does not verify")
            }
            IpError::TimeExceeded => f.write_str("the time to live reached zero"),
            IpError::WouldFragment { length, mtu } => write!(
                f,
                "{length} bytes do not fit an MTU of {mtu} and may not be fragmented"
            ),
            IpError::NoRoute => f.write_str("no route reaches the destination"),
            IpError::OverlappingFragment => {
                f.write_str("two fragments claim the same bytes and disagree")
            }
            IpError::TooLarge(length) => {
                write!(f, "a datagram of {length} bytes does not fit the buffer")
            }
            IpError::BadIcmp => f.write_str("the ICMP message is short or does not verify"),
        }
    }
}
