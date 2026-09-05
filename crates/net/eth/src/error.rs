// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a frame or an ARP packet could not be read or written.
//!
//! What is *not* here: a frame that is not for this station, and a frame
//! carrying a type no layer above reads. Those are drops and not errors —
//! see [`receive`](crate::frame::receive).

use core::fmt;

use net_wire::WireError;

/// What this crate found wrong with the bytes it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EthError {
    /// A cursor ran out of buffer. The inner error says how much was
    /// wanted and how much there was.
    Wire(WireError),
    /// The frame is shorter than the fourteen bytes of its header.
    FrameTooShort(usize),
    /// The payload is longer than the MTU of 1500 bytes.
    PayloadTooLong(usize),
    /// The ARP packet does not describe Ethernet and IPv4: the hardware
    /// type, the protocol type, or one of the two address lengths is not
    /// the one this crate resolves for.
    NotEthernetIpv4,
    /// The ARP operation is neither a request nor a reply.
    UnknownOperation(u16),
}

impl From<WireError> for EthError {
    fn from(error: WireError) -> EthError {
        EthError::Wire(error)
    }
}

impl fmt::Display for EthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EthError::Wire(error) => error.fmt(f),
            EthError::FrameTooShort(length) => {
                write!(f, "a frame of {length} bytes is shorter than its header")
            }
            EthError::PayloadTooLong(length) => {
                write!(f, "a payload of {length} bytes is longer than the MTU")
            }
            EthError::NotEthernetIpv4 => f.write_str("the ARP packet is not for Ethernet and IPv4"),
            EthError::UnknownOperation(operation) => {
                write!(f, "{operation} is neither an ARP request nor a reply")
            }
        }
    }
}
