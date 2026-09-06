// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every error says what it found, once.

use crypto_rng::{EntropyError, RngError};
use net_udp::UdpError;
use net_wire::{Ipv4Addr, WireError};

use crate::error::DhcpError;
use crate::option::OptionCode;

#[test]
fn every_error_reads_as_a_sentence() {
    let cases = [
        (
            DhcpError::Wire(WireError::OutOfBounds {
                needed: 240,
                available: 12,
            }),
            "240 bytes were needed and 12 are left",
        ),
        (
            DhcpError::Rng(RngError::Entropy(EntropyError::Unavailable)),
            "reseeding failed: the entropy source is unavailable",
        ),
        (
            DhcpError::Udp(UdpError::UnspecifiedPort),
            "port zero names no service",
        ),
        (
            DhcpError::Cookie([0, 1, 2, 3]),
            "[0, 1, 2, 3] is not the magic cookie an option block begins with",
        ),
        (
            DhcpError::Hardware { htype: 6, hlen: 8 },
            "a link of type 6 with 8-byte addresses is not one this client speaks over",
        ),
        (DhcpError::Op(3), "3 is neither a request nor a reply"),
        (
            DhcpError::TruncatedOption(OptionCode::SUBNET_MASK),
            "option 1 reaches past the block",
        ),
        (DhcpError::MissingEnd, "the option block has no end marker"),
        (
            DhcpError::MissingMessageType,
            "the message carries no message type",
        ),
        (
            DhcpError::MissingOption(OptionCode::LEASE_TIME),
            "the message carries no option 51",
        ),
        (
            DhcpError::OptionLength {
                code: OptionCode::MESSAGE_TYPE,
                len: 3,
            },
            "a body of 3 bytes is not option 53",
        ),
        (
            DhcpError::Netmask(Ipv4Addr::new(255, 0, 255, 0)),
            "the bits of the mask 255.0.255.0 do not run together",
        ),
        (
            DhcpError::Address(Ipv4Addr::UNSPECIFIED),
            "0.0.0.0 is not an address a host can be given",
        ),
        (
            DhcpError::TooLong(256),
            "a body of 256 bytes is longer than an option carries",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn the_errors_of_the_crates_below_arrive_unchanged() {
    let wire = WireError::MixedFamilies;
    assert_eq!(DhcpError::from(wire), DhcpError::Wire(wire));
    let rng = RngError::Exhausted;
    assert_eq!(DhcpError::from(rng), DhcpError::Rng(rng));
    let udp = UdpError::NoPort;
    assert_eq!(DhcpError::from(udp), DhcpError::Udp(udp));
}
