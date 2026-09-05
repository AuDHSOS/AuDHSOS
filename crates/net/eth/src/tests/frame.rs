// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Frames: what is read, what is written, and what is dropped without a
//! word.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "a test computes a length or an instant in the open"
)]

use net_wire::{EtherType, Ipv6Addr, MacAddr, Reader, WireError, Writer};
use test_support::generators::bytes;
use test_support::property::check;

use crate::error::EthError;
use crate::frame::{Frame, HEADER_LEN, MAX_FRAME_LEN, MTU, multicast_hardware, receive};

/// The address of the station under test.
const INTERFACE: MacAddr = MacAddr::new([0x02, 0x00, 0x5E, 0x00, 0x00, 0x01]);

/// Some other station.
const PEER: MacAddr = MacAddr::new([0x02, 0x00, 0x5E, 0x00, 0x00, 0x02]);

/// A frame with `payload_len` bytes behind the header, addressed to the
/// interface and carrying IPv4.
fn frame_of(payload_len: usize) -> Vec<u8> {
    let mut buffer = vec![0u8; HEADER_LEN + payload_len];
    let mut writer = Writer::new(&mut buffer);
    Frame::write(
        &mut writer,
        INTERFACE,
        PEER,
        EtherType::IPV4,
        &vec![0xA5; payload_len],
    )
    .expect("a frame of a length a frame may have");
    buffer
}

#[test]
fn a_frame_of_the_minimum_length_is_a_header_and_nothing_else() {
    let bytes = frame_of(0);
    assert_eq!(bytes.len(), HEADER_LEN);
    let frame = Frame::parse(&bytes).expect("fourteen bytes are a frame");
    assert_eq!(frame.destination(), INTERFACE);
    assert_eq!(frame.source(), PEER);
    assert_eq!(frame.ether_type(), EtherType::IPV4);
    assert_eq!(frame.payload(), &[]);
}

#[test]
fn a_frame_of_the_maximum_length_carries_the_whole_mtu() {
    let bytes = frame_of(MTU);
    assert_eq!(bytes.len(), MAX_FRAME_LEN);
    let frame = Frame::parse(&bytes).expect("a frame of the maximum length");
    assert_eq!(frame.payload().len(), MTU);
    assert!(frame.payload().iter().all(|byte| *byte == 0xA5));
}

#[test]
fn one_byte_short_of_a_header_is_not_a_frame() {
    let bytes = frame_of(0);
    let short = bytes.get(..HEADER_LEN - 1).expect("thirteen bytes");
    assert_eq!(Frame::parse(short), Err(EthError::FrameTooShort(13)));
    assert_eq!(Frame::parse(&[]), Err(EthError::FrameTooShort(0)));
    assert_eq!(receive(short, INTERFACE), None);
}

#[test]
fn one_byte_past_the_mtu_is_not_a_frame_either() {
    let mut bytes = frame_of(MTU);
    bytes.push(0);
    assert_eq!(Frame::parse(&bytes), Err(EthError::PayloadTooLong(MTU + 1)));
    assert_eq!(receive(&bytes, INTERFACE), None);
}

#[test]
fn the_destination_filter_takes_this_station_the_broadcast_and_a_group() {
    for (destination, accepted) in [
        (INTERFACE, true),
        (MacAddr::BROADCAST, true),
        (MacAddr::new([0x33, 0x33, 0xFF, 0x00, 0x00, 0x01]), true),
        (PEER, false),
        (MacAddr::UNSPECIFIED, false),
    ] {
        let mut buffer = [0u8; HEADER_LEN];
        let mut writer = Writer::new(&mut buffer);
        Frame::write(&mut writer, destination, PEER, EtherType::IPV4, &[])
            .expect("a header fits in a header");
        let frame = Frame::parse(&buffer).expect("a frame");
        assert_eq!(frame.is_for(INTERFACE), accepted, "{destination}");
        assert_eq!(
            receive(&buffer, INTERFACE).is_some(),
            accepted,
            "{destination}"
        );
    }
}

#[test]
fn a_type_no_layer_here_reads_is_dropped_and_not_reported() {
    let mut buffer = [0u8; HEADER_LEN];
    let mut writer = Writer::new(&mut buffer);
    Frame::write(&mut writer, INTERFACE, PEER, EtherType::new(0x8100), &[])
        .expect("a header fits in a header");
    // It parses: the bytes are a frame, and nothing about them is wrong.
    let frame = Frame::parse(&buffer).expect("a frame");
    assert_eq!(frame.ether_type().get(), 0x8100);
    assert!(frame.is_for(INTERFACE));
    // It is simply not for this system.
    assert_eq!(receive(&buffer, INTERFACE), None);
}

#[test]
fn the_types_this_system_reads_pass_the_filter() {
    for ether_type in [EtherType::IPV4, EtherType::IPV6, EtherType::ARP] {
        let mut buffer = [0u8; HEADER_LEN];
        let mut writer = Writer::new(&mut buffer);
        Frame::write(&mut writer, INTERFACE, PEER, ether_type, &[]).expect("a header");
        assert!(receive(&buffer, INTERFACE).is_some(), "{ether_type}");
    }
}

#[test]
fn a_frame_that_does_not_fit_writes_nothing() {
    let mut buffer = [0u8; HEADER_LEN + 3];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        Frame::write(&mut writer, INTERFACE, PEER, EtherType::IPV4, &[1, 2, 3, 4]),
        Err(EthError::Wire(WireError::OutOfBounds {
            needed: HEADER_LEN + 4,
            available: HEADER_LEN + 3,
        }))
    );
    assert_eq!(writer.position(), 0);
    assert_eq!(buffer, [0u8; HEADER_LEN + 3]);
}

#[test]
fn a_payload_past_the_mtu_is_refused_before_the_buffer_is_measured() {
    let payload = vec![0u8; MTU + 1];
    let mut buffer = vec![0u8; MAX_FRAME_LEN + 1];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        Frame::write(&mut writer, INTERFACE, PEER, EtherType::IPV4, &payload),
        Err(EthError::PayloadTooLong(MTU + 1))
    );
    assert_eq!(writer.position(), 0);
}

#[test]
fn the_header_is_written_in_the_order_a_reader_expects() {
    let mut buffer = [0u8; HEADER_LEN + 2];
    let mut writer = Writer::new(&mut buffer);
    Frame::write(
        &mut writer,
        MacAddr::BROADCAST,
        PEER,
        EtherType::ARP,
        &[0xDE, 0xAD],
    )
    .expect("room for the frame");
    let mut reader = Reader::new(&buffer);
    assert_eq!(reader.read_mac(), Ok(MacAddr::BROADCAST));
    assert_eq!(reader.read_mac(), Ok(PEER));
    assert_eq!(reader.read_u16(), Ok(EtherType::ARP.get()));
    assert_eq!(reader.rest(), &[0xDE, 0xAD]);
}

#[test]
fn what_is_written_is_what_is_read_back() {
    check("frame round trip", &bytes(0..=200), |payload| {
        let mut buffer = vec![0u8; HEADER_LEN + payload.len()];
        let mut writer = Writer::new(&mut buffer);
        Frame::write(&mut writer, INTERFACE, PEER, EtherType::IPV4, payload)
            .map_err(|error| error.to_string())?;
        let frame = Frame::parse(&buffer).map_err(|error| error.to_string())?;
        if frame.payload() == payload.as_slice()
            && frame.destination() == INTERFACE
            && frame.source() == PEER
        {
            Ok(())
        } else {
            Err(format!("{} bytes came back changed", payload.len()))
        }
    });
}

#[test]
fn no_sequence_of_bytes_makes_the_parser_do_anything_but_answer() {
    check("frame parse is total", &bytes(0..=64), |input| {
        // Whatever the bytes are, both entry points answer.
        let parsed = Frame::parse(input);
        let received = receive(input, INTERFACE);
        if parsed.is_err() && received.is_some() {
            return Err("a frame that does not parse was received".to_owned());
        }
        Ok(())
    });
}

#[test]
fn an_ipv6_group_is_reached_at_the_address_rfc_2464_derives() {
    // RFC 2464, section 7: the bytes `33:33` and the last four of the
    // group. This is what lets Neighbor Discovery address a station whose
    // hardware address nobody knows yet, where ARP has to broadcast.
    let all_nodes = multicast_hardware(Ipv6Addr::ALL_NODES);
    assert_eq!(
        all_nodes,
        MacAddr::new([0x33, 0x33, 0x00, 0x00, 0x00, 0x01])
    );
    assert!(all_nodes.is_multicast());
    assert!(!all_nodes.is_broadcast());

    let host = Ipv6Addr::from_octets([
        0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0x02, 0x1b, 0x44, 0xff, 0xfe, 0x11, 0x3a, 0xb7,
    ]);
    let group = host.solicited_node();
    assert_eq!(
        multicast_hardware(group),
        MacAddr::new([0x33, 0x33, 0xFF, 0x11, 0x3A, 0xB7]),
        "the low twenty-four bits of the address, and the byte above them"
    );
    // Two hosts whose addresses differ above the low twenty-four bits
    // share the group, and therefore the frame; the layer above sorts
    // them out by the target field of the solicitation.
    let other = Ipv6Addr::from_octets([
        0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe, 0x11, 0x3a, 0xb7,
    ]);
    assert_eq!(
        multicast_hardware(other.solicited_node()),
        multicast_hardware(group)
    );
}
