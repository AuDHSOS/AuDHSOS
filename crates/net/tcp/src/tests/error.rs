// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every error says what it found, once.

use crypto_rng::{EntropyError, RngError};
use net_wire::{Port, WireError};

use crate::error::TcpError;
use crate::state::State;

#[test]
fn every_error_reads_as_a_sentence() {
    let cases = [
        (
            TcpError::Wire(WireError::OutOfBounds {
                needed: 20,
                available: 3,
            }),
            "20 bytes were needed and 3 are left",
        ),
        (TcpError::Short(7), "7 bytes are fewer than a header takes"),
        (
            TcpError::DataOffset(4),
            "a data offset of 4 words is not one this segment has",
        ),
        (
            TcpError::Checksum(0x1234),
            "the checksum 0x1234 does not verify",
        ),
        (TcpError::BadOption(30), "option 30 is not a whole option"),
        (
            TcpError::MixedFamilies,
            "the two addresses are not of one family",
        ),
        (
            TcpError::TooLarge(70_000),
            "70000 bytes do not fit the buffer",
        ),
        (
            TcpError::WrongState(State::Listen),
            "LISTEN does not allow that",
        ),
        (TcpError::Reset, "the peer reset the connection"),
        (
            TcpError::PortInUse(Port::new(80)),
            "port 80 is already bound",
        ),
        (TcpError::NoConnection, "the connection table is full"),
        (
            TcpError::UnknownConnection,
            "that identifier names no open connection",
        ),
        (
            TcpError::Rng(RngError::Entropy(EntropyError::Unavailable)),
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
    assert_eq!(TcpError::from(wire), TcpError::Wire(wire));
    let rng = RngError::Exhausted;
    assert_eq!(TcpError::from(rng), TcpError::Rng(rng));
}
