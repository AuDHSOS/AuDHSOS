// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a datagram could not be read or written, and why a socket could not
//! be opened.

use core::fmt;

use crypto_rng::RngError;
use net_wire::{Port, WireError};

/// What this crate found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UdpError {
    /// A cursor ran out of buffer.
    Wire(WireError),
    /// The length field is below the eight bytes a header takes, or beyond
    /// the bytes there are. The value is the one the datagram carried.
    Length(u16),
    /// The checksum does not verify. The value is the one the datagram
    /// carried.
    Checksum(u16),
    /// The checksum field is zero over IPv6, where RFC 8200, section 8.1
    /// makes it mandatory, or a caller asked for it to be omitted there.
    ChecksumRequired,
    /// The two addresses are not of one family, so there is no
    /// pseudo-header to sum over.
    MixedFamilies,
    /// A socket already holds that port.
    PortInUse(Port),
    /// Port zero names no service and is not one a socket may hold. A
    /// caller that wants any port asks for an ephemeral one.
    UnspecifiedPort,
    /// Every port of the dynamic range that the search visited is taken.
    NoPort,
    /// The socket table is full.
    NoSocket,
    /// The identifier names no open socket.
    UnknownSocket,
    /// The payload is longer than the length field can express.
    TooLarge(usize),
    /// The generator an ephemeral port was to be drawn from failed.
    Rng(RngError),
}

impl From<WireError> for UdpError {
    fn from(error: WireError) -> UdpError {
        UdpError::Wire(error)
    }
}

impl From<RngError> for UdpError {
    fn from(error: RngError) -> UdpError {
        UdpError::Rng(error)
    }
}

impl fmt::Display for UdpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UdpError::Wire(error) => error.fmt(f),
            UdpError::Length(length) => {
                write!(f, "a length of {length} is not one this datagram has")
            }
            UdpError::Checksum(carried) => {
                write!(f, "the checksum {carried:#06x} does not verify")
            }
            UdpError::ChecksumRequired => {
                f.write_str("a datagram over IPv6 carries a checksum or none at all")
            }
            UdpError::MixedFamilies => f.write_str("the two addresses are not of one family"),
            UdpError::PortInUse(port) => write!(f, "port {} is already bound", port.get()),
            UdpError::UnspecifiedPort => f.write_str("port zero names no service"),
            UdpError::NoPort => f.write_str("no port of the dynamic range is free"),
            UdpError::NoSocket => f.write_str("the socket table is full"),
            UdpError::UnknownSocket => f.write_str("that identifier names no open socket"),
            UdpError::TooLarge(length) => {
                write!(
                    f,
                    "a payload of {length} bytes is longer than a datagram carries"
                )
            }
            UdpError::Rng(error) => error.fmt(f),
        }
    }
}
