// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every error says what went wrong in one line.

use net_wire::WireError;

use crate::error::EthError;

#[test]
fn every_variant_writes_a_sentence_of_its_own() {
    let wire = WireError::OutOfBounds {
        needed: 28,
        available: 4,
    };
    let messages = [
        (EthError::Wire(wire), "28 bytes were needed and 4 are left"),
        (
            EthError::FrameTooShort(13),
            "a frame of 13 bytes is shorter than its header",
        ),
        (
            EthError::PayloadTooLong(1501),
            "a payload of 1501 bytes is longer than the MTU",
        ),
        (
            EthError::NotEthernetIpv4,
            "the ARP packet is not for Ethernet and IPv4",
        ),
        (
            EthError::UnknownOperation(3),
            "3 is neither an ARP request nor a reply",
        ),
    ];
    for (error, expected) in messages {
        assert_eq!(error.to_string(), expected, "{error:?}");
    }
}

#[test]
fn a_wire_error_arrives_through_the_question_mark() {
    let wire = WireError::Address;
    assert_eq!(EthError::from(wire), EthError::Wire(wire));
    assert_ne!(EthError::from(wire), EthError::NotEthernetIpv4);
    assert!(format!("{:?}", EthError::from(wire)).contains("Wire"));
}
