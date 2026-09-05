// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a packet could not be read, written, or delivered.

use core::fmt;

use net_ip::IpError;
use net_wire::WireError;

/// What this crate found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ipv6Error {
    /// A cursor ran out of buffer.
    Wire(WireError),
    /// The routing table, the reassembler, or the fragmenter of `net-ip`
    /// said no. Those three are shared with IPv4 and answer in their own
    /// error type rather than in a copy of it.
    Ip(IpError),
    /// The version field is not six.
    Version(u8),
    /// The payload length field reaches past the bytes that arrived.
    PayloadLength(u16),
    /// The payload is longer than the sixteen bits of the length field.
    PayloadTooLong(usize),
    /// An extension header declares a length that runs past the packet.
    /// The value is how far it reached.
    ExtensionLength(usize),
    /// The extension header chain is longer than this host walks, in
    /// headers or in bytes. A chain has no end of its own, so the bound is
    /// the end.
    ChainTooLong {
        /// How many headers had been stepped over.
        headers: usize,
        /// How many bytes they had taken.
        bytes: usize,
    },
    /// The `ICMPv6` message is shorter than its header, or its checksum
    /// does not verify over the pseudo-header.
    BadIcmp,
    /// A Neighbor Discovery option of length zero, which RFC 4861,
    /// section 4.6 requires a receiver to discard the packet over, or one
    /// that runs past the message. The value is the option type.
    BadOption(u8),
    /// The `ICMPv6` message is not one of the four Neighbor Discovery
    /// messages this host reads, or it did not arrive the way RFC 4861,
    /// section 7.1 requires one to. The value is the message type.
    NotDiscovery(u8),
    /// The packet is longer than the MTU and could not be cut up, because
    /// the MTU has no room for a header and eight bytes behind it.
    WouldFragment {
        /// How long the payload is.
        length: usize,
        /// What the path carries.
        mtu: usize,
    },
}

impl From<WireError> for Ipv6Error {
    fn from(error: WireError) -> Ipv6Error {
        Ipv6Error::Wire(error)
    }
}

impl From<IpError> for Ipv6Error {
    fn from(error: IpError) -> Ipv6Error {
        Ipv6Error::Ip(error)
    }
}

impl fmt::Display for Ipv6Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ipv6Error::Wire(error) => error.fmt(f),
            Ipv6Error::Ip(error) => error.fmt(f),
            Ipv6Error::Version(version) => write!(f, "the version is {version} and not six"),
            Ipv6Error::PayloadLength(length) => {
                write!(f, "a payload length of {length} is not one this packet has")
            }
            Ipv6Error::PayloadTooLong(length) => {
                write!(f, "{length} bytes do not fit in the payload length field")
            }
            Ipv6Error::ExtensionLength(reach) => {
                write!(
                    f,
                    "an extension header reaches to {reach} and the packet does not"
                )
            }
            Ipv6Error::ChainTooLong { headers, bytes } => write!(
                f,
                "a chain of {headers} headers over {bytes} bytes is longer than this host walks"
            ),
            Ipv6Error::BadIcmp => f.write_str("the ICMPv6 message is short or does not verify"),
            Ipv6Error::BadOption(kind) => {
                write!(
                    f,
                    "the option of type {kind} has no length or runs past the message"
                )
            }
            Ipv6Error::NotDiscovery(message_type) => {
                write!(f, "type {message_type} is not a Neighbor Discovery message")
            }
            Ipv6Error::WouldFragment { length, mtu } => {
                write!(
                    f,
                    "{length} bytes do not fit an MTU of {mtu} and may not be cut up"
                )
            }
        }
    }
}
