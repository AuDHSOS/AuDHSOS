// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why an operation on the stack did not happen.
//!
//! The errors of the layers below arrive whole. What is added here is the
//! handful of things only the facade can find wrong: a handle that names
//! a socket that is gone, a table with no room, an interface with no
//! address, and a name that resolved to nothing.

use core::fmt;

use crypto_rng::RngError;
use net_dhcp::DhcpError;
use net_dns::DnsError;
use net_eth::EthError;
use net_ip::IpError;
use net_ipv6::Ipv6Error;
use net_tcp::TcpError;
use net_udp::UdpError;
use net_wire::WireError;

/// What this crate, or a layer under it, found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StackError {
    /// A cursor ran out of buffer.
    Wire(WireError),
    /// The frame layer.
    Eth(EthError),
    /// IPv4.
    Ip(IpError),
    /// IPv6.
    Ipv6(Ipv6Error),
    /// UDP.
    Udp(UdpError),
    /// TCP.
    Tcp(TcpError),
    /// The resolver.
    Dns(DnsError),
    /// The address configuration client.
    Dhcp(DhcpError),
    /// A generator failed.
    Rng(RngError),
    /// The handle names a socket or a connection that was closed. The
    /// index is still there; the generation behind it is not.
    Stale,
    /// The handle names no slot at all.
    Unknown,
    /// Every slot of the table is in use.
    Full,
    /// This host has no address of the family the operation needs, so
    /// there is nothing for a packet to leave from.
    NoAddress,
    /// The address table is full.
    TooManyAddresses,
    /// A name resolved to no address this host can reach.
    Unresolved,
    /// A connection by name or by list is already running, and there is
    /// room for one.
    Busy,
    /// Every candidate address was tried and none answered.
    Unreachable,
}

macro_rules! from {
    ($from:ty, $variant:ident) => {
        impl From<$from> for StackError {
            fn from(error: $from) -> StackError {
                StackError::$variant(error)
            }
        }
    };
}

from!(WireError, Wire);
from!(EthError, Eth);
from!(IpError, Ip);
from!(Ipv6Error, Ipv6);
from!(UdpError, Udp);
from!(TcpError, Tcp);
from!(DnsError, Dns);
from!(DhcpError, Dhcp);
from!(RngError, Rng);

impl fmt::Display for StackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StackError::Wire(error) => error.fmt(f),
            StackError::Eth(error) => error.fmt(f),
            StackError::Ip(error) => error.fmt(f),
            StackError::Ipv6(error) => error.fmt(f),
            StackError::Udp(error) => error.fmt(f),
            StackError::Tcp(error) => error.fmt(f),
            StackError::Dns(error) => error.fmt(f),
            StackError::Dhcp(error) => error.fmt(f),
            StackError::Rng(error) => error.fmt(f),
            StackError::Stale => f.write_str("that handle names a socket that was closed"),
            StackError::Unknown => f.write_str("that handle names no slot"),
            StackError::Full => f.write_str("every slot of the table is in use"),
            StackError::NoAddress => f.write_str("this host has no address of that family"),
            StackError::TooManyAddresses => f.write_str("the address table is full"),
            StackError::Unresolved => f.write_str("the name resolved to no address"),
            StackError::Busy => f.write_str("a connection is already being made"),
            StackError::Unreachable => f.write_str("no candidate address answered"),
        }
    }
}
