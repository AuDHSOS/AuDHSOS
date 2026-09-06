// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every error says what it found, once, and the errors of the layers
//! below arrive whole.

use crypto_rng::RngError;
use net_wire::WireError;

use crate::error::StackError;

#[test]
fn every_error_of_this_crates_own_reads_as_a_sentence() {
    let cases = [
        (
            StackError::Stale,
            "that handle names a socket that was closed",
        ),
        (StackError::Unknown, "that handle names no slot"),
        (StackError::Full, "every slot of the table is in use"),
        (
            StackError::NoAddress,
            "this host has no address of that family",
        ),
        (StackError::TooManyAddresses, "the address table is full"),
        (StackError::Unresolved, "the name resolved to no address"),
        (StackError::Busy, "a connection is already being made"),
        (StackError::Unreachable, "no candidate address answered"),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn every_layer_below_says_its_own_thing() {
    let cases: [(StackError, &str); 9] = [
        (
            StackError::from(WireError::MixedFamilies),
            "the two addresses are of different families",
        ),
        (
            StackError::from(net_eth::EthError::PayloadTooLong(2000)),
            "a payload of 2000 bytes is longer than the MTU",
        ),
        (
            StackError::from(net_ip::IpError::NoRoute),
            "no route reaches the destination",
        ),
        (
            StackError::from(net_ipv6::Ipv6Error::BadIcmp),
            "the ICMPv6 message is short or does not verify",
        ),
        (
            StackError::from(net_udp::UdpError::NoPort),
            "no port of the dynamic range is free",
        ),
        (
            StackError::from(net_tcp::TcpError::NoConnection),
            "the connection table is full",
        ),
        (
            StackError::from(net_dns::DnsError::NoServer),
            "no server was given",
        ),
        (
            StackError::from(net_dhcp::DhcpError::MissingEnd),
            "the option block has no end marker",
        ),
        (
            StackError::from(RngError::Exhausted),
            "the scripted generator has no bytes left",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text, "{error:?}");
    }
}
