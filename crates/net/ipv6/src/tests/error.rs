// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every error says what went wrong in one line.

use net_ip::IpError;
use net_wire::WireError;

use crate::error::Ipv6Error;

#[test]
fn every_variant_writes_a_sentence_of_its_own() {
    let messages = [
        (
            Ipv6Error::Wire(WireError::OutOfBounds {
                needed: 40,
                available: 4,
            }),
            "40 bytes were needed and 4 are left",
        ),
        (
            Ipv6Error::Ip(IpError::NoRoute),
            "no route reaches the destination",
        ),
        (Ipv6Error::Version(4), "the version is 4 and not six"),
        (
            Ipv6Error::PayloadLength(1500),
            "a payload length of 1500 is not one this packet has",
        ),
        (
            Ipv6Error::PayloadTooLong(65536),
            "65536 bytes do not fit in the payload length field",
        ),
        (
            Ipv6Error::ExtensionLength(128),
            "an extension header reaches to 128 and the packet does not",
        ),
        (
            Ipv6Error::ChainTooLong {
                headers: 9,
                bytes: 72,
            },
            "a chain of 9 headers over 72 bytes is longer than this host walks",
        ),
        (
            Ipv6Error::BadIcmp,
            "the ICMPv6 message is short or does not verify",
        ),
        (
            Ipv6Error::BadOption(1),
            "the option of type 1 has no length or runs past the message",
        ),
        (
            Ipv6Error::NotDiscovery(137),
            "type 137 is not a Neighbor Discovery message",
        ),
        (
            Ipv6Error::WouldFragment {
                length: 2000,
                mtu: 48,
            },
            "2000 bytes do not fit an MTU of 48 and may not be cut up",
        ),
    ];
    for (error, expected) in messages {
        assert_eq!(error.to_string(), expected, "{error:?}");
    }
}

#[test]
fn the_errors_of_the_crates_below_arrive_through_the_question_mark() {
    let wire = WireError::Address;
    assert_eq!(Ipv6Error::from(wire), Ipv6Error::Wire(wire));
    let ip = IpError::OverlappingFragment;
    assert_eq!(Ipv6Error::from(ip), Ipv6Error::Ip(ip));
    assert_ne!(Ipv6Error::from(wire), Ipv6Error::BadIcmp);
    assert!(format!("{:?}", Ipv6Error::BadIcmp).contains("BadIcmp"));
}
