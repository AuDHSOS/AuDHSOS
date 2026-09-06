// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every error says what it found, once.

use crypto_rng::{EntropyError, RngError};
use net_udp::UdpError;
use net_wire::WireError;

use crate::error::DnsError;
use crate::message::ResponseCode;
use crate::record::RecordType;

#[test]
fn every_error_reads_as_a_sentence() {
    let cases = [
        (
            DnsError::Wire(WireError::OutOfBounds {
                needed: 12,
                available: 4,
            }),
            "12 bytes were needed and 4 are left",
        ),
        (
            DnsError::Rng(RngError::Entropy(EntropyError::Unavailable)),
            "reseeding failed: the entropy source is unavailable",
        ),
        (
            DnsError::Udp(UdpError::UnspecifiedPort),
            "port zero names no service",
        ),
        (
            DnsError::Label(64),
            "a label of 64 bytes is not one a name has",
        ),
        (
            DnsError::NameTooLong,
            "a name is at most 255 bytes on the wire",
        ),
        (DnsError::Text, "the text is not a domain name"),
        (
            DnsError::LabelKind(0x40),
            "the length octet 0x40 names no kind of label",
        ),
        (
            DnsError::PointerForward { at: 12, to: 30 },
            "the pointer at 12 points to 30 and not backwards",
        ),
        (
            DnsError::PointerChain,
            "a name of more compression jumps than are read",
        ),
        (
            DnsError::Rdata {
                record_type: RecordType::A,
                len: 5,
            },
            "a body of 5 bytes is not a record of type 1",
        ),
        (
            DnsError::Truncated,
            "the answer was truncated and there is no TCP here",
        ),
        (
            DnsError::Rcode(ResponseCode::SERVER_FAILURE),
            "the server answered server failure",
        ),
        (
            DnsError::CnameLoop,
            "the alias chain returns to a record it has used",
        ),
        (
            DnsError::CnameChain,
            "the alias chain is longer than eight links",
        ),
        (
            DnsError::MixedFamilies,
            "the server is not of this resolver's family",
        ),
        (DnsError::NoServer, "no server was given"),
        (DnsError::TooManyServers, "the server table is full"),
        (DnsError::Busy, "a resolution is already running"),
        (
            DnsError::Exhausted,
            "every attempt at every server went unanswered",
        ),
        (
            DnsError::Deadline,
            "the deadline passed before an answer came",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn the_errors_of_the_crates_below_arrive_unchanged() {
    let wire = WireError::MixedFamilies;
    assert_eq!(DnsError::from(wire), DnsError::Wire(wire));
    let rng = RngError::Exhausted;
    assert_eq!(DnsError::from(rng), DnsError::Rng(rng));
    let udp = UdpError::NoPort;
    assert_eq!(DnsError::from(udp), DnsError::Udp(udp));
}
