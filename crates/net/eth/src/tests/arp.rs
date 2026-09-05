// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! ARP against the field layout of RFC 826.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "a test computes a length or an instant in the open"
)]

use net_wire::{Ipv4Addr, MacAddr, WireError, Writer};
use test_support::generators::bytes;
use test_support::property::check;

use crate::arp::{Operation, PACKET_LEN, Packet, respond};
use crate::error::EthError;

/// This station.
const INTERFACE: MacAddr = MacAddr::new([0x00, 0x1B, 0x44, 0x11, 0x3A, 0xB7]);

/// Its address.
const ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 7);

/// The station asking.
const PEER: MacAddr = MacAddr::new([0x00, 0x1B, 0x44, 0x11, 0x3A, 0xB8]);

/// Its address.
const PEER_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);

/// A request from the peer for this station's address, byte for byte in
/// the field order of RFC 826: hardware type, protocol type, the two
/// address lengths, the operation, and then sender hardware, sender
/// protocol, target hardware, target protocol.
const REQUEST_BYTES: [u8; PACKET_LEN] = [
    0x00, 0x01, // hardware type: Ethernet
    0x08, 0x00, // protocol type: IPv4
    0x06, // hardware address length
    0x04, // protocol address length
    0x00, 0x01, // operation: request
    0x00, 0x1B, 0x44, 0x11, 0x3A, 0xB8, // sender hardware
    192, 168, 1, 1, // sender protocol
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // target hardware, unset in a request
    192, 168, 1, 7, // target protocol
];

#[test]
fn a_request_reads_as_the_fields_rfc_826_names() {
    let packet = Packet::parse(&REQUEST_BYTES).expect("a well-formed request");
    assert_eq!(packet.operation, Operation::Request);
    assert_eq!(packet.sender_hardware, PEER);
    assert_eq!(packet.sender_protocol, PEER_ADDRESS);
    assert_eq!(packet.target_hardware, MacAddr::UNSPECIFIED);
    assert_eq!(packet.target_protocol, ADDRESS);
    assert!(!packet.is_gratuitous());
}

#[test]
fn a_request_writes_the_bytes_it_was_read_from() {
    let packet = Packet::request(PEER, PEER_ADDRESS, ADDRESS);
    let mut buffer = [0u8; PACKET_LEN];
    let mut writer = Writer::new(&mut buffer);
    packet.write(&mut writer).expect("room for a packet");
    assert_eq!(writer.position(), PACKET_LEN);
    assert_eq!(buffer, REQUEST_BYTES);
    assert_eq!(Packet::parse(&buffer), Ok(packet));
}

#[test]
fn a_reply_answers_the_request_by_turning_it_around() {
    let request = Packet::parse(&REQUEST_BYTES).expect("a request");
    let reply = Packet::reply_to(&request, INTERFACE);
    assert_eq!(reply.operation, Operation::Reply);
    assert_eq!(reply.sender_hardware, INTERFACE);
    assert_eq!(reply.sender_protocol, ADDRESS);
    assert_eq!(reply.target_hardware, PEER);
    assert_eq!(reply.target_protocol, PEER_ADDRESS);

    let mut buffer = [0u8; PACKET_LEN];
    let mut writer = Writer::new(&mut buffer);
    reply.write(&mut writer).expect("room for a packet");
    assert_eq!(Packet::parse(&buffer), Ok(reply));
}

#[test]
fn a_request_for_this_station_produces_exactly_one_reply() {
    let request = Packet::parse(&REQUEST_BYTES).expect("a request");
    let answer = respond(&request, INTERFACE, ADDRESS).expect("this station holds the address");
    assert_eq!(answer, Packet::reply_to(&request, INTERFACE));
    // Answering the answer is what a loop is made of, and there is none.
    assert_eq!(respond(&answer, PEER, PEER_ADDRESS), None);
}

#[test]
fn a_request_for_another_address_produces_none() {
    let request = Packet::parse(&REQUEST_BYTES).expect("a request");
    assert_eq!(
        respond(&request, INTERFACE, Ipv4Addr::new(192, 168, 1, 9)),
        None
    );
    assert_eq!(respond(&request, INTERFACE, Ipv4Addr::UNSPECIFIED), None);
}

#[test]
fn a_reply_is_never_answered() {
    let request = Packet::parse(&REQUEST_BYTES).expect("a request");
    let reply = Packet::reply_to(&request, INTERFACE);
    assert_eq!(respond(&reply, INTERFACE, ADDRESS), None);
}

#[test]
fn a_gratuitous_request_names_its_sender_on_both_sides() {
    let mut packet = Packet::request(PEER, PEER_ADDRESS, PEER_ADDRESS);
    assert!(packet.is_gratuitous());
    packet.target_protocol = ADDRESS;
    assert!(!packet.is_gratuitous());
}

#[test]
fn a_packet_for_other_address_spaces_is_refused() {
    for (offset, value) in [
        (1usize, 6u8), // hardware type: not Ethernet
        (3, 0x86),     // protocol type: not IPv4
        (4, 8),        // hardware address length: not six
        (5, 16),       // protocol address length: not four
    ] {
        let mut bytes = REQUEST_BYTES;
        let slot = bytes.get_mut(offset).expect("an offset inside the packet");
        *slot = value;
        assert_eq!(
            Packet::parse(&bytes),
            Err(EthError::NotEthernetIpv4),
            "offset {offset}"
        );
    }
}

#[test]
fn an_operation_that_is_neither_a_request_nor_a_reply_is_refused() {
    let mut bytes = REQUEST_BYTES;
    let slot = bytes.get_mut(7).expect("the low byte of the operation");
    *slot = 3;
    assert_eq!(Packet::parse(&bytes), Err(EthError::UnknownOperation(3)));
    assert_eq!(Operation::new(0), Err(EthError::UnknownOperation(0)));
    assert_eq!(Operation::new(1), Ok(Operation::Request));
    assert_eq!(Operation::new(2), Ok(Operation::Reply));
    assert_eq!(Operation::Request.get(), 1);
    assert_eq!(Operation::Reply.get(), 2);
}

#[test]
fn a_packet_shorter_than_its_fields_is_refused() {
    for length in 0..PACKET_LEN {
        let short = REQUEST_BYTES.get(..length).expect("a prefix");
        assert!(
            matches!(Packet::parse(short), Err(EthError::Wire(_)))
                || matches!(Packet::parse(short), Err(EthError::NotEthernetIpv4)),
            "{length} bytes"
        );
    }
}

#[test]
fn a_packet_that_does_not_fit_writes_nothing() {
    let packet = Packet::request(PEER, PEER_ADDRESS, ADDRESS);
    let mut buffer = [0u8; PACKET_LEN - 1];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        packet.write(&mut writer),
        Err(EthError::Wire(WireError::OutOfBounds {
            needed: PACKET_LEN,
            available: PACKET_LEN - 1,
        }))
    );
    assert_eq!(writer.position(), 0);
    assert_eq!(buffer, [0u8; PACKET_LEN - 1]);
}

#[test]
fn no_sequence_of_bytes_makes_the_parser_do_anything_but_answer() {
    check(
        "arp parse is total",
        &bytes(0..=64),
        |input| match Packet::parse(input) {
            Ok(packet) => {
                let mut buffer = [0u8; PACKET_LEN];
                let mut writer = Writer::new(&mut buffer);
                packet
                    .write(&mut writer)
                    .map_err(|error| error.to_string())?;
                if Packet::parse(&buffer) == Ok(packet) {
                    Ok(())
                } else {
                    Err("a packet did not re-encode to itself".to_owned())
                }
            }
            Err(_) => Ok(()),
        },
    );
}

#[test]
fn a_gratuitous_claim_on_this_station_s_own_address_is_answered() {
    // Somebody announces the address this station holds. The packet is a
    // request whose target is that address, so `respond` answers it, and
    // the answer is this station saying the address is its own. Defending
    // an address is what RFC 826 has the owner do; deciding what to make
    // of the conflict belongs above this crate.
    let claim = Packet::request(PEER, ADDRESS, ADDRESS);
    assert!(claim.is_gratuitous());
    let answer = respond(&claim, INTERFACE, ADDRESS).expect("this station holds the address");
    assert_eq!(answer.sender_hardware, INTERFACE);
    assert_eq!(answer.sender_protocol, ADDRESS);
    assert_eq!(answer.target_hardware, PEER);
}

#[test]
fn a_gratuitous_claim_on_another_address_is_not_answered() {
    let claim = Packet::request(PEER, PEER_ADDRESS, PEER_ADDRESS);
    assert_eq!(respond(&claim, INTERFACE, ADDRESS), None);
}
