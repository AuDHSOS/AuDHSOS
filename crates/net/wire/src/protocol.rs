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
    /// IPv6.
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
        matches!(self, EtherType::IPV4 | EtherType::IPV6 | EtherType::ARP)
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

/// The protocol field of an IPv4 header, which is the next-header field of
/// an IPv6 header: one table, as IANA keeps it. The values that name an
/// IPv6 extension header rather than an upper-layer protocol are the ones
/// [`is_extension_header`](Protocol::is_extension_header) reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Protocol(u8);

impl Protocol {
    /// The IPv6 hop-by-hop options header, which is an extension header
    /// and, being the value zero, the one that cannot be confused with an
    /// absent field.
    pub const HOP_BY_HOP: Protocol = Protocol(0);
    /// ICMP for IPv4.
    pub const ICMP: Protocol = Protocol(1);
    /// TCP.
    pub const TCP: Protocol = Protocol(6);
    /// UDP.
    pub const UDP: Protocol = Protocol(17);
    /// The IPv6 routing header, an extension header. This system sends
    /// none and reads one only to step over it.
    pub const ROUTING: Protocol = Protocol(43);
    /// The IPv6 fragment header, an extension header.
    pub const FRAGMENT: Protocol = Protocol(44);
    /// ICMP for IPv6, which carries Neighbor Discovery and the router
    /// advertisements a host configures itself from.
    pub const ICMPV6: Protocol = Protocol(58);
    /// No next header: the packet ends here, and nothing follows the
    /// header that says so.
    pub const NO_NEXT_HEADER: Protocol = Protocol(59);
    /// The IPv6 destination options header, an extension header.
    pub const DESTINATION_OPTIONS: Protocol = Protocol(60);

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
        matches!(
            self,
            Protocol::ICMP | Protocol::ICMPV6 | Protocol::TCP | Protocol::UDP
        )
    }

    /// Whether the value names an IPv6 extension header rather than an
    /// upper-layer protocol. A parser walks the chain while this holds and
    /// hands what follows to the layer above when it stops.
    #[must_use]
    pub const fn is_extension_header(self) -> bool {
        matches!(
            self,
            Protocol::HOP_BY_HOP
                | Protocol::ROUTING
                | Protocol::FRAGMENT
                | Protocol::DESTINATION_OPTIONS
        )
    }

    /// Whether the protocol's checksum covers the pseudo-header of the two
    /// addresses. `ICMPv4`'s does not and `ICMPv6`'s does — RFC 4443,
    /// section 2.3 changed that, and it is the one difference a checksum
    /// routine has to know before it sums an ICMP message of either
    /// family.
    #[must_use]
    pub const fn has_pseudo_header(self) -> bool {
        matches!(self, Protocol::TCP | Protocol::UDP | Protocol::ICMPV6)
    }

    /// The name of the protocol, for a diagnostic, or `None` for one this
    /// crate does not name.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        match self {
            Protocol::HOP_BY_HOP => Some("hop-by-hop options"),
            Protocol::ICMP => Some("ICMP"),
            Protocol::TCP => Some("TCP"),
            Protocol::UDP => Some("UDP"),
            Protocol::ROUTING => Some("routing header"),
            Protocol::FRAGMENT => Some("fragment header"),
            Protocol::ICMPV6 => Some("ICMPv6"),
            Protocol::NO_NEXT_HEADER => Some("no next header"),
            Protocol::DESTINATION_OPTIONS => Some("destination options"),
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
