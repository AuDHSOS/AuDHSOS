// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two demultiplexing tables: what an Ethernet frame carries, and what
//! an IPv4 datagram carries.
//!
//! Both are wrappers over the number on the wire and not enumerations, so
//! that a value this system has no layer for is a value and not a parse
//! error. Ethernet drops a frame whose type is not one it registered,
//! rather than rejecting it loudly, and a wrapper is what lets the frame
//! be read far enough to make that decision.

use core::fmt;

/// The type field of an Ethernet II frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EtherType(u16);

impl EtherType {
    /// IPv4.
    pub const IPV4: EtherType = EtherType(0x0800);
    /// ARP.
    pub const ARP: EtherType = EtherType(0x0806);
    /// IPv6, which this system does not carry (D-50) and which is named
    /// here so that a frame of it is dropped as a known type rather than
    /// puzzled over.
    pub const IPV6: EtherType = EtherType(0x86DD);

    /// The smallest value that is a type. Below it, the field is the
    /// length of an IEEE 802.3 frame, which this system does not read.
    pub const MIN: EtherType = EtherType(0x0600);

    /// The type of this number.
    #[must_use]
    pub const fn new(number: u16) -> EtherType {
        EtherType(number)
    }

    /// The number.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Whether a layer of this system reads frames of this type.
    #[must_use]
    pub const fn is_registered(self) -> bool {
        matches!(self, EtherType::IPV4 | EtherType::ARP)
    }

    /// Whether the field is a type at all rather than a length.
    #[must_use]
    pub const fn is_type(self) -> bool {
        self.0 >= EtherType::MIN.0
    }

    /// The name of the type, for a diagnostic, or `None` for one this
    /// crate does not name.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        match self {
            EtherType::IPV4 => Some("IPv4"),
            EtherType::ARP => Some("ARP"),
            EtherType::IPV6 => Some("IPv6"),
            _ => None,
        }
    }
}

impl fmt::Display for EtherType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "0x{:04x}", self.0),
        }
    }
}

/// The protocol field of an IPv4 header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Protocol(u8);

impl Protocol {
    /// ICMP for IPv4.
    pub const ICMP: Protocol = Protocol(1);
    /// TCP.
    pub const TCP: Protocol = Protocol(6);
    /// UDP.
    pub const UDP: Protocol = Protocol(17);

    /// The protocol of this number.
    #[must_use]
    pub const fn new(number: u8) -> Protocol {
        Protocol(number)
    }

    /// The number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Whether a layer of this system reads datagrams of this protocol.
    #[must_use]
    pub const fn is_registered(self) -> bool {
        matches!(self, Protocol::ICMP | Protocol::TCP | Protocol::UDP)
    }

    /// Whether the protocol's checksum covers the pseudo-header of the two
    /// addresses. ICMP's does not, which is the one difference that
    /// matters when a checksum is computed for it.
    #[must_use]
    pub const fn has_pseudo_header(self) -> bool {
        matches!(self, Protocol::TCP | Protocol::UDP)
    }

    /// The name of the protocol, for a diagnostic, or `None` for one this
    /// crate does not name.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        match self {
            Protocol::ICMP => Some("ICMP"),
            Protocol::TCP => Some("TCP"),
            Protocol::UDP => Some("UDP"),
            _ => None,
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "{}", self.0),
        }
    }
}
