// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every error says what it found, once.

use crypto_rng::{EntropyError, RngError};
use net_wire::{Port, WireError};

use crate::error::UdpError;

#[test]
fn every_error_reads_as_a_sentence() {
    let cases = [
        (
            UdpError::Wire(WireError::OutOfBounds {
                needed: 8,
                available: 3,
            }),
            "8 bytes were needed and 3 are left",
        ),
        (
            UdpError::Length(7),
            "a length of 7 is not one this datagram has",
        ),
        (
            UdpError::Checksum(0x1234),
            "the checksum 0x1234 does not verify",
        ),
        (
            UdpError::ChecksumRequired,
            "a datagram over IPv6 carries a checksum or none at all",
        ),
        (
            UdpError::MixedFamilies,
            "the two addresses are not of one family",
        ),
        (
            UdpError::PortInUse(Port::new(53)),
            "port 53 is already bound",
        ),
        (UdpError::UnspecifiedPort, "port zero names no service"),
        (UdpError::NoPort, "no port of the dynamic range is free"),
        (UdpError::NoSocket, "the socket table is full"),
        (
            UdpError::UnknownSocket,
            "that identifier names no open socket",
        ),
        (
            UdpError::TooLarge(70_000),
            "a payload of 70000 bytes is longer than a datagram carries",
        ),
        (
            UdpError::Rng(RngError::Entropy(EntropyError::Unavailable)),
            "reseeding failed: the entropy source is unavailable",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn the_errors_of_the_crates_below_arrive_unchanged() {
    let wire = WireError::MixedFamilies;
    assert_eq!(UdpError::from(wire), UdpError::Wire(wire));
    let rng = RngError::Exhausted;
    assert_eq!(UdpError::from(rng), UdpError::Rng(rng));
}
