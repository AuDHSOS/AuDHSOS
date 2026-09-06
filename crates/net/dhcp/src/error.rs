// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a message could not be read or written, and why an offer could not
//! be turned into a lease.

use core::fmt;

use crypto_rng::RngError;
use net_udp::UdpError;
use net_wire::{Ipv4Addr, WireError};

use crate::option::OptionCode;

/// What this crate found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DhcpError {
    /// A cursor ran out of buffer.
    Wire(WireError),
    /// The generator a transaction id or a jitter was to be drawn from
    /// failed.
    Rng(RngError),
    /// The datagram around the message could not be written.
    Udp(UdpError),
    /// The four bytes where RFC 2132, section 2 puts its magic cookie are
    /// other bytes, so what follows them is not an option block.
    Cookie([u8; 4]),
    /// A hardware type or length that is not the six bytes of Ethernet.
    /// This client speaks over one kind of link.
    Hardware {
        /// The type the message carried.
        htype: u8,
        /// The length it carried.
        hlen: u8,
    },
    /// An op code that is neither a request nor a reply.
    Op(u8),
    /// An option whose length reaches past the block it stands in.
    TruncatedOption(OptionCode),
    /// The option block runs out without the end marker of RFC 2132,
    /// section 3.2.
    MissingEnd,
    /// No message type, which RFC 2131, section 4.1 makes the one option
    /// every DHCP message carries.
    MissingMessageType,
    /// An option a lease cannot be made without. RFC 2131, section 4.3.1
    /// requires the lease time and the server identifier in an
    /// acknowledgment, and a mask is what turns the address into a
    /// network.
    MissingOption(OptionCode),
    /// An option whose body is not the length its code has. The two values
    /// are the code and the length it carried.
    OptionLength {
        /// Which option.
        code: OptionCode,
        /// How long its body was.
        len: usize,
    },
    /// A subnet mask whose bits do not run together, which is no prefix
    /// and therefore no network.
    Netmask(Ipv4Addr),
    /// An address no host can be given: the unspecified one, a broadcast,
    /// a multicast group, or a loopback address.
    Address(Ipv4Addr),
    /// An option body longer than the length field can express.
    TooLong(usize),
}

impl From<WireError> for DhcpError {
    fn from(error: WireError) -> DhcpError {
        DhcpError::Wire(error)
    }
}

impl From<RngError> for DhcpError {
    fn from(error: RngError) -> DhcpError {
        DhcpError::Rng(error)
    }
}

impl From<UdpError> for DhcpError {
    fn from(error: UdpError) -> DhcpError {
        DhcpError::Udp(error)
    }
}

impl fmt::Display for DhcpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DhcpError::Wire(error) => error.fmt(f),
            DhcpError::Rng(error) => error.fmt(f),
            DhcpError::Udp(error) => error.fmt(f),
            DhcpError::Cookie(bytes) => {
                write!(
                    f,
                    "{bytes:?} is not the magic cookie an option block begins with"
                )
            }
            DhcpError::Hardware { htype, hlen } => {
                write!(
                    f,
                    "a link of type {htype} with {hlen}-byte addresses is not one this client speaks over"
                )
            }
            DhcpError::Op(op) => write!(f, "{op} is neither a request nor a reply"),
            DhcpError::TruncatedOption(code) => {
                write!(f, "option {} reaches past the block", code.get())
            }
            DhcpError::MissingEnd => f.write_str("the option block has no end marker"),
            DhcpError::MissingMessageType => f.write_str("the message carries no message type"),
            DhcpError::MissingOption(code) => {
                write!(f, "the message carries no option {}", code.get())
            }
            DhcpError::OptionLength { code, len } => {
                write!(f, "a body of {len} bytes is not option {}", code.get())
            }
            DhcpError::Netmask(mask) => {
                write!(f, "the bits of the mask {mask} do not run together")
            }
            DhcpError::Address(address) => {
                write!(f, "{address} is not an address a host can be given")
            }
            DhcpError::TooLong(len) => {
                write!(f, "a body of {len} bytes is longer than an option carries")
            }
        }
    }
}
