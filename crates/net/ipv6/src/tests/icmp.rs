// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ICMPv6` against the message formats of RFC 4443, and the one thing
//! that separates it from `ICMPv4`: a checksum that covers the
//! pseudo-header of both addresses.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a message at known offsets"
)]

use net_wire::{Ipv6Addr, Protocol, Writer, transport_v6};
use test_support::generators::bytes;
use test_support::property::check;

use super::{HOST, PEER, checksummed, packet};
use crate::error::Ipv6Error;
use crate::header::{HEADER_LEN as PACKET_HEADER_LEN, MIN_MTU, Packet};
use crate::icmp::{HEADER_LEN, Message, Unreachable, may_answer_with_error, quoted_len, verify};

/// The payload an echo carries here.
const ECHO_DATA: [u8; 8] = [0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68];

/// An echo request from this host to the other end, with its checksum
/// filled in over the pseudo-header the two addresses make.
fn echo_request(source: Ipv6Addr, destination: Ipv6Addr) -> Vec<u8> {
    let mut message = vec![128, 0, 0, 0, 0x12, 0x34, 0x00, 0x01];
    message.extend_from_slice(&ECHO_DATA);
    checksummed(source, destination, &mut message);
    message
}

#[test]
fn an_echo_request_reads_as_the_fields_rfc_4443_names() {
    let message = echo_request(HOST, PEER);
    assert_eq!(
        Message::parse_checked(HOST, PEER, &message),
        Ok(Message::EchoRequest {
            identifier: 0x1234,
            sequence: 1,
            payload: &ECHO_DATA,
        })
    );
}

#[test]
fn an_echo_request_is_answered_with_the_same_payload() {
    let message = echo_request(PEER, HOST);
    let request = Message::parse_checked(PEER, HOST, &message).expect("a well-formed request");
    let reply = request.reply().expect("an echo request deserves a reply");
    assert_eq!(
        reply,
        Message::EchoReply {
            identifier: 0x1234,
            sequence: 1,
            payload: &ECHO_DATA,
        }
    );
    // The reply goes back the way it came, so the pseudo-header swaps.
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    reply.write(HOST, PEER, &mut writer).expect("room");
    let written = writer.written().to_vec();
    assert_eq!(written[0], 129, "the reply is type 129");
    assert!(
        verify(HOST, PEER, &written),
        "a reply whose checksum does not verify at the receiver"
    );
    assert_eq!(Message::parse_checked(HOST, PEER, &written), Ok(reply));
}

#[test]
fn nothing_but_an_echo_request_deserves_a_reply() {
    for message in [
        Message::EchoReply {
            identifier: 0,
            sequence: 0,
            payload: &[],
        },
        Message::DestinationUnreachable {
            reason: Unreachable::Port,
            quoted: &[],
        },
        Message::PacketTooBig {
            mtu: 1280,
            quoted: &[],
        },
    ] {
        assert_eq!(message.reply(), None);
    }
}

#[test]
fn the_checksum_covers_the_pseudo_header_where_icmpv4_has_none() {
    let message = echo_request(HOST, PEER);
    // The bytes verify between the addresses they were summed for.
    assert!(verify(HOST, PEER, &message));
    // And not between others: this is exactly what RFC 4443, section 2.3
    // added and what `ICMPv4` does not have. A message that arrived with
    // a forged address is caught here and nowhere else, since the header
    // it came in has no checksum of its own.
    let third = Ipv6Addr::from_octets([
        0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x03,
    ]);
    assert!(!verify(HOST, third, &message));
    assert_eq!(
        Message::parse_checked(HOST, third, &message),
        Err(Ipv6Error::BadIcmp)
    );
    // What it does not catch is the two addresses swapped. The sum is
    // commutative, so the pseudo-header binds a message to the pair and
    // not to its direction; what tells a reply from a request is the
    // type field, which is inside the sum.
    assert!(verify(PEER, HOST, &message));
    // The structure is still readable, which is what the fuzz target and
    // a caller holding a message without its packet use.
    assert!(Message::parse(&message).is_ok());
}

#[test]
fn a_corrupted_message_does_not_verify() {
    let mut message = echo_request(HOST, PEER);
    message[HEADER_LEN] ^= 0x01;
    assert!(!verify(HOST, PEER, &message));
}

#[test]
fn a_message_shorter_than_its_header_is_refused() {
    for length in 0..HEADER_LEN {
        let short = vec![0u8; length];
        assert_eq!(Message::parse(&short), Err(Ipv6Error::BadIcmp), "{length}");
        assert!(!verify(HOST, PEER, &short), "{length}");
    }
}

#[test]
fn a_packet_too_big_is_read_with_the_mtu_it_reports() {
    let mut message = vec![2, 0, 0, 0, 0x00, 0x00, 0x05, 0xDC];
    message.extend_from_slice(b"the packet that did not fit");
    checksummed(PEER, HOST, &mut message);
    let parsed = Message::parse_checked(PEER, HOST, &message).expect("a well-formed message");
    let Message::PacketTooBig { mtu, quoted } = parsed else {
        panic!("a packet too big reads as one");
    };
    assert_eq!(mtu, 1500);
    assert_eq!(quoted, b"the packet that did not fit");
    assert!(parsed.is_error());
}

#[test]
fn a_packet_too_big_writes_the_mtu_back_where_it_was_read() {
    let message = Message::PacketTooBig {
        mtu: 1280,
        quoted: b"quoted",
    };
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    message.write(HOST, PEER, &mut writer).expect("room");
    let written = writer.written().to_vec();
    assert_eq!(&written[4..8], &1280u32.to_be_bytes());
    assert_eq!(Message::parse_checked(HOST, PEER, &written), Ok(message));
}

#[test]
fn destination_unreachable_and_time_exceeded_read_and_write_back() {
    let messages = [
        Message::DestinationUnreachable {
            reason: Unreachable::Port,
            quoted: b"eight by",
        },
        Message::DestinationUnreachable {
            reason: Unreachable::Other(9),
            quoted: &[],
        },
        Message::TimeExceeded {
            code: 1,
            quoted: b"a fragment",
        },
    ];
    for message in messages {
        let mut buffer = [0u8; 64];
        let mut writer = Writer::new(&mut buffer);
        message.write(HOST, PEER, &mut writer).expect("room");
        let written = writer.written().to_vec();
        assert_eq!(
            Message::parse_checked(HOST, PEER, &written),
            Ok(message),
            "{message:?}"
        );
        assert!(message.is_error(), "{message:?}");
    }
}

#[test]
fn every_unreachable_code_rfc_4443_names_round_trips() {
    for code in 0..=8u8 {
        let reason = Unreachable::new(code);
        assert_eq!(reason.get(), code);
    }
    assert_eq!(Unreachable::new(0), Unreachable::NoRoute);
    assert_eq!(Unreachable::new(1), Unreachable::Prohibited);
    assert_eq!(Unreachable::new(2), Unreachable::BeyondScope);
    assert_eq!(Unreachable::new(3), Unreachable::Address);
    assert_eq!(Unreachable::new(4), Unreachable::Port);
    assert_eq!(Unreachable::new(5), Unreachable::SourcePolicy);
    assert_eq!(Unreachable::new(6), Unreachable::RejectRoute);
    assert_eq!(Unreachable::new(7), Unreachable::Other(7));
}

#[test]
fn a_type_this_crate_does_not_name_is_carried_and_not_written() {
    // 135 is a neighbor solicitation, which `crate::ndp` reads and this
    // module carries.
    let mut message = vec![135, 0, 0, 0, 0, 0, 0, 0];
    message.extend_from_slice(&PEER.octets());
    checksummed(PEER, HOST, &mut message);
    let parsed = Message::parse_checked(PEER, HOST, &message).expect("a well-formed message");
    let Message::Other {
        message_type,
        code,
        body,
    } = parsed
    else {
        panic!("an unnamed type is carried");
    };
    assert_eq!(message_type, 135);
    assert_eq!(code, 0);
    assert_eq!(body, &PEER.octets());
    // Informational, so not an error.
    assert!(!parsed.is_error());
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        parsed.write(HOST, PEER, &mut writer),
        Err(Ipv6Error::BadIcmp)
    );
}

#[test]
fn an_error_of_a_type_this_crate_does_not_name_is_still_an_error() {
    // Type 4 is a parameter problem: an error this host does not
    // generate and does read, because RFC 4443, section 2.4 (a) has one
    // passed up to the process whose packet caused it.
    let message = vec![4, 0, 0, 0, 0, 0, 0, 0];
    let parsed = Message::parse(&message).expect("eight bytes");
    assert!(parsed.is_error());
}

#[test]
fn a_buffer_too_small_for_a_message_takes_nothing() {
    let message = Message::EchoRequest {
        identifier: 1,
        sequence: 1,
        payload: &ECHO_DATA,
    };
    let mut buffer = [0u8; HEADER_LEN + ECHO_DATA.len() - 1];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(
        message.write(HOST, PEER, &mut writer),
        Err(Ipv6Error::Wire(_))
    ));
    assert_eq!(writer.position(), 0);
}

#[test]
fn an_error_quotes_as_much_as_fits_the_minimum_mtu() {
    let room = MIN_MTU - PACKET_HEADER_LEN - HEADER_LEN;
    assert_eq!(quoted_len(0), 0);
    assert_eq!(quoted_len(64), 64);
    assert_eq!(quoted_len(room), room);
    // RFC 4443, section 2.4 (c): as much as possible without the error
    // itself exceeding the minimum MTU.
    assert_eq!(quoted_len(MIN_MTU), room);
    assert_eq!(quoted_len(usize::MAX), room);
    assert!(PACKET_HEADER_LEN + HEADER_LEN + quoted_len(usize::MAX) <= MIN_MTU);
}

/// A packet from `source` to `destination` carrying `message` as its
/// `ICMPv6` payload.
fn icmp_packet(source: Ipv6Addr, destination: Ipv6Addr, message: &[u8]) -> Vec<u8> {
    packet(source, destination, Protocol::ICMPV6, 64, message)
}

#[test]
fn no_error_answers_an_error() {
    let mut message = vec![1, 4, 0, 0, 0, 0, 0, 0];
    checksummed(PEER, HOST, &mut message);
    let bytes = icmp_packet(PEER, HOST, &message);
    let packet = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = packet.upper_layer().expect("no chain");
    assert!(!may_answer_with_error(packet, upper, false));
}

#[test]
fn an_error_answers_a_question() {
    let message = echo_request(PEER, HOST);
    let bytes = icmp_packet(PEER, HOST, &message);
    let packet = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = packet.upper_layer().expect("no chain");
    assert!(may_answer_with_error(packet, upper, false));
}

#[test]
fn no_error_answers_a_multicast_destination_or_a_multicast_frame() {
    let message = echo_request(PEER, Ipv6Addr::ALL_NODES);
    let bytes = icmp_packet(PEER, Ipv6Addr::ALL_NODES, &message);
    let packet = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = packet.upper_layer().expect("no chain");
    assert!(!may_answer_with_error(packet, upper, false));

    // The same packet addressed to this host, but carried in a frame the
    // link delivered to everybody.
    let message = echo_request(PEER, HOST);
    let bytes = icmp_packet(PEER, HOST, &message);
    let packet = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = packet.upper_layer().expect("no chain");
    assert!(!may_answer_with_error(packet, upper, true));
}

#[test]
fn no_error_answers_a_source_that_names_no_single_node() {
    for source in [
        Ipv6Addr::UNSPECIFIED,
        Ipv6Addr::ALL_NODES,
        Ipv6Addr::LOCALHOST,
    ] {
        let message = echo_request(source, HOST);
        let bytes = icmp_packet(source, HOST, &message);
        let packet = Packet::parse(&bytes).expect("a well-formed packet");
        let upper = packet.upper_layer().expect("no chain");
        assert!(!may_answer_with_error(packet, upper, false), "{source}");
    }
}

#[test]
fn no_byte_stream_makes_the_message_parser_panic() {
    check("icmpv6_message_parse", &bytes(0..=64), |input| {
        let Ok(message) = Message::parse(input) else {
            return Ok(());
        };
        if matches!(message, Message::Other { .. }) {
            return Ok(());
        }
        let mut buffer = [0u8; 128];
        let mut writer = Writer::new(&mut buffer);
        if message.write(HOST, PEER, &mut writer).is_err() {
            return Ok(());
        }
        let written = writer.written();
        if Message::parse(written) != Ok(message) {
            return Err("a message that did not read back as itself".into());
        }
        if transport_v6(HOST, PEER, Protocol::ICMPV6, written) != Ok(0) {
            return Err("a message this crate wrote whose checksum does not verify".into());
        }
        Ok(())
    });
}
