// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every error says what went wrong in one line.

use net_wire::WireError;

use crate::error::IpError;

#[test]
fn every_variant_writes_a_sentence_of_its_own() {
    let messages = [
        (
            IpError::Wire(WireError::OutOfBounds {
                needed: 20,
                available: 4,
            }),
            "20 bytes were needed and 4 are left",
        ),
        (IpError::Version(6), "the version is 6 and not four"),
        (
            IpError::HeaderLength(4),
            "a header of 4 words is not one this system reads",
        ),
        (
            IpError::TotalLength(19),
            "a total length of 19 is not one this datagram has",
        ),
        (
            IpError::Checksum(0x0A89),
            "the header checksum 0x0a89 does not verify",
        ),
        (IpError::TimeExceeded, "the time to live reached zero"),
        (
            IpError::WouldFragment {
                length: 2000,
                mtu: 1500,
            },
            "2000 bytes do not fit an MTU of 1500 and may not be fragmented",
        ),
        (IpError::NoRoute, "no route reaches the destination"),
        (
            IpError::OverlappingFragment,
            "two fragments claim the same bytes and disagree",
        ),
        (
            IpError::TooLarge(70000),
            "a datagram of 70000 bytes does not fit the buffer",
        ),
        (
            IpError::BadIcmp,
            "the ICMP message is short or does not verify",
        ),
    ];
    for (error, expected) in messages {
        assert_eq!(error.to_string(), expected, "{error:?}");
    }
}

#[test]
fn a_wire_error_arrives_through_the_question_mark() {
    let wire = WireError::Address;
    assert_eq!(IpError::from(wire), IpError::Wire(wire));
    assert_ne!(IpError::from(wire), IpError::NoRoute);
    assert!(format!("{:?}", IpError::NoRoute).contains("NoRoute"));
}
