// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The IPv6 header against the field layout of RFC 8200, section 3, and
//! the extension header chain of its section 4.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a header at known offsets"
)]

use net_wire::{Ipv6Addr, Protocol, Writer};
use test_support::generators::bytes;
use test_support::property::check;

use super::{HOST, PEER, extension, fragment_header, packet, short_extension};
use crate::error::Ipv6Error;
use crate::header::{
    DEFAULT_HOP_LIMIT, Fragment, FragmentHeader, HEADER_LEN, Header, MAX_EXTENSION_BYTES,
    MAX_EXTENSION_HEADERS, Packet, VERSION,
};

/// The eight bytes of payload most packets here carry.
const PAYLOAD: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// A packet from this host to the other end, carrying UDP.
fn udp_packet() -> Vec<u8> {
    packet(HOST, PEER, Protocol::UDP, DEFAULT_HOP_LIMIT, &PAYLOAD)
}

#[test]
fn a_packet_reads_as_the_fields_rfc_8200_names() {
    let bytes = udp_packet();
    let packet = Packet::parse(&bytes).expect("a well-formed packet");
    assert_eq!(packet.source(), HOST);
    assert_eq!(packet.destination(), PEER);
    assert_eq!(packet.next_header(), Protocol::UDP);
    assert_eq!(packet.hop_limit(), DEFAULT_HOP_LIMIT);
    assert!(!packet.is_expired());
    assert_eq!(packet.header().len(), HEADER_LEN);
    assert_eq!(packet.payload(), &PAYLOAD);
    assert_eq!(packet.total_len(), HEADER_LEN + PAYLOAD.len());
}

#[test]
fn a_packet_of_the_minimum_length_carries_no_payload() {
    let bytes = packet(HOST, PEER, Protocol::NO_NEXT_HEADER, 1, &[]);
    assert_eq!(bytes.len(), HEADER_LEN);
    let packet = Packet::parse(&bytes).expect("a header and nothing behind it");
    assert!(packet.payload().is_empty());
    assert_eq!(packet.total_len(), HEADER_LEN);
}

#[test]
fn a_packet_one_byte_short_of_the_header_is_refused() {
    let bytes = packet(HOST, PEER, Protocol::UDP, 1, &[]);
    let short = &bytes[..HEADER_LEN - 1];
    assert!(matches!(
        Packet::parse(short),
        Err(Ipv6Error::Wire(_) | Ipv6Error::PayloadLength(0))
    ));
}

#[test]
fn a_payload_length_the_buffer_does_not_hold_is_refused() {
    let mut bytes = udp_packet();
    // Claim one byte more than arrived.
    let claimed = u16::try_from(PAYLOAD.len() + 1).expect("a small number");
    bytes[4..6].copy_from_slice(&claimed.to_be_bytes());
    assert_eq!(
        Packet::parse(&bytes),
        Err(Ipv6Error::PayloadLength(claimed))
    );
}

#[test]
fn a_payload_length_shorter_than_the_buffer_cuts_the_payload() {
    let mut bytes = udp_packet();
    bytes[5] = 4;
    let packet = Packet::parse(&bytes).expect("a packet with trailing bytes");
    // A link pads a short frame, and the padding is not payload.
    assert_eq!(packet.payload(), &PAYLOAD[..4]);
}

#[test]
fn a_version_that_is_not_six_is_refused() {
    let mut bytes = udp_packet();
    bytes[0] = 0x45;
    assert_eq!(Packet::parse(&bytes), Err(Ipv6Error::Version(4)));
}

#[test]
fn a_hop_limit_of_zero_is_read_and_reported_as_expired() {
    let bytes = packet(HOST, PEER, Protocol::UDP, 0, &PAYLOAD);
    let packet = Packet::parse(&bytes).expect("a packet out of hops");
    assert_eq!(packet.hop_limit(), 0);
    // It is read, not dropped: a packet addressed to this host arrives
    // with whatever is left, and nothing here forwards.
    assert!(packet.is_expired());
    assert_eq!(packet.payload(), &PAYLOAD);
}

#[test]
fn there_is_no_header_checksum_and_therefore_nothing_to_verify() {
    let mut corrupt = udp_packet();
    // Flip a byte of the source address. In IPv4 this is what the header
    // checksum catches; here there is no such field, and the packet
    // parses. What catches it is the upper-layer checksum, which
    // RFC 8200, section 8.1 sums over both addresses.
    corrupt[8] ^= 0xFF;
    let packet = Packet::parse(&corrupt).expect("a corrupted header still parses");
    assert_ne!(packet.source(), HOST);
    assert_eq!(packet.header().len(), HEADER_LEN);
}

#[test]
fn a_packet_without_extension_headers_reaches_the_upper_layer_at_once() {
    let bytes = udp_packet();
    let packet = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = packet.upper_layer().expect("no chain to walk");
    assert_eq!(upper.protocol, Protocol::UDP);
    assert_eq!(upper.payload, &PAYLOAD);
    assert_eq!(upper.fragment, None);
    assert_eq!(upper.headers, 0);
    assert_eq!(upper.bytes, 0);
    assert!(!upper.is_fragment());
}

#[test]
fn each_of_the_four_extension_headers_is_stepped_over() {
    for kind in [
        Protocol::HOP_BY_HOP,
        Protocol::ROUTING,
        Protocol::DESTINATION_OPTIONS,
    ] {
        let mut body = short_extension(Protocol::UDP);
        body.extend_from_slice(&PAYLOAD);
        let bytes = packet(HOST, PEER, kind, DEFAULT_HOP_LIMIT, &body);
        let packet = Packet::parse(&bytes).expect("a packet with one extension header");
        let upper = packet.upper_layer().expect("a chain of one");
        assert_eq!(upper.protocol, Protocol::UDP, "{kind}");
        assert_eq!(upper.payload, &PAYLOAD, "{kind}");
        assert_eq!(upper.headers, 1);
        assert_eq!(upper.bytes, 8);
    }
    // The fragment header is the fourth, and the one whose contents are
    // kept rather than skipped.
    let mut body = fragment_header(Protocol::UDP, 0, false, 0xDEAD_BEEF);
    body.extend_from_slice(&PAYLOAD);
    let bytes = packet(HOST, PEER, Protocol::FRAGMENT, DEFAULT_HOP_LIMIT, &body);
    let packet = Packet::parse(&bytes).expect("a packet with a fragment header");
    let upper = packet.upper_layer().expect("a chain of one");
    assert_eq!(upper.protocol, Protocol::UDP);
    assert_eq!(
        upper.fragment,
        Some(Fragment {
            offset: 0,
            more: false,
            identification: 0xDEAD_BEEF,
        })
    );
    // Offset zero and no more: an atomic fragment, which names a whole
    // datagram (RFC 8200, section 4.5).
    assert!(!upper.is_fragment());
}

#[test]
fn the_four_in_the_order_rfc_8200_recommends_are_walked_to_the_end() {
    let mut body = short_extension(Protocol::ROUTING);
    body.extend_from_slice(&short_extension(Protocol::FRAGMENT));
    body.extend_from_slice(&fragment_header(Protocol::DESTINATION_OPTIONS, 16, true, 7));
    body.extend_from_slice(&short_extension(Protocol::TCP));
    body.extend_from_slice(&PAYLOAD);
    let bytes = packet(HOST, PEER, Protocol::HOP_BY_HOP, DEFAULT_HOP_LIMIT, &body);
    let packet = Packet::parse(&bytes).expect("a packet with four extension headers");
    let upper = packet.upper_layer().expect("a chain of four");
    assert_eq!(upper.protocol, Protocol::TCP);
    assert_eq!(upper.payload, &PAYLOAD);
    assert_eq!(upper.headers, 4);
    assert_eq!(upper.bytes, 32);
    assert_eq!(
        upper.fragment,
        Some(Fragment {
            offset: 16,
            more: true,
            identification: 7,
        })
    );
    assert!(upper.is_fragment());
}

#[test]
fn a_header_that_repeats_is_stepped_over_like_any_other() {
    let mut body = short_extension(Protocol::DESTINATION_OPTIONS);
    body.extend_from_slice(&short_extension(Protocol::UDP));
    body.extend_from_slice(&PAYLOAD);
    let bytes = packet(
        HOST,
        PEER,
        Protocol::DESTINATION_OPTIONS,
        DEFAULT_HOP_LIMIT,
        &body,
    );
    let packet = Packet::parse(&bytes).expect("a repeated header");
    let upper = packet.upper_layer().expect("a chain of two");
    assert_eq!(upper.protocol, Protocol::UDP);
    assert_eq!(upper.headers, 2);
}

#[test]
fn a_second_fragment_header_is_stepped_over_and_the_first_is_kept() {
    let mut body = fragment_header(Protocol::FRAGMENT, 8, true, 1);
    body.extend_from_slice(&fragment_header(Protocol::UDP, 24, false, 2));
    body.extend_from_slice(&PAYLOAD);
    let bytes = packet(HOST, PEER, Protocol::FRAGMENT, DEFAULT_HOP_LIMIT, &body);
    let packet = Packet::parse(&bytes).expect("two fragment headers");
    let upper = packet.upper_layer().expect("a chain of two");
    assert_eq!(
        upper.fragment,
        Some(Fragment {
            offset: 8,
            more: true,
            identification: 1,
        })
    );
}

#[test]
fn a_chain_of_more_headers_than_the_bound_is_dropped() {
    let mut body = Vec::new();
    for _ in 0..=MAX_EXTENSION_HEADERS {
        body.extend_from_slice(&short_extension(Protocol::HOP_BY_HOP));
    }
    body.extend_from_slice(&short_extension(Protocol::UDP));
    body.extend_from_slice(&PAYLOAD);
    let bytes = packet(HOST, PEER, Protocol::HOP_BY_HOP, DEFAULT_HOP_LIMIT, &body);
    let packet = Packet::parse(&bytes).expect("a long chain still parses as a packet");
    assert_eq!(
        packet.upper_layer(),
        Err(Ipv6Error::ChainTooLong {
            headers: MAX_EXTENSION_HEADERS + 1,
            bytes: MAX_EXTENSION_HEADERS * 8,
        })
    );
}

#[test]
fn a_chain_of_more_bytes_than_the_bound_is_dropped() {
    // Three headers of 256 bytes: fewer than the header bound, more than
    // the byte bound.
    let long = 256usize;
    let units = u8::try_from(long / 8 - 1).expect("thirty-one units");
    let mut body = Vec::new();
    for _ in 0..3 {
        body.extend_from_slice(&extension(Protocol::HOP_BY_HOP, units, long));
    }
    body.extend_from_slice(&short_extension(Protocol::UDP));
    body.extend_from_slice(&PAYLOAD);
    let bytes = packet(HOST, PEER, Protocol::HOP_BY_HOP, DEFAULT_HOP_LIMIT, &body);
    let packet = Packet::parse(&bytes).expect("a fat chain still parses as a packet");
    let error = packet.upper_layer().expect_err("more bytes than the bound");
    assert!(
        matches!(error, Ipv6Error::ChainTooLong { headers, bytes } if headers == 3 && bytes > MAX_EXTENSION_BYTES),
        "{error:?}"
    );
}

#[test]
fn an_extension_header_that_reaches_past_the_packet_is_dropped() {
    // One header that declares 128 bytes in a packet that has 16.
    let mut body = extension(Protocol::UDP, 15, 16);
    body.extend_from_slice(&PAYLOAD);
    let bytes = packet(HOST, PEER, Protocol::HOP_BY_HOP, DEFAULT_HOP_LIMIT, &body);
    let packet = Packet::parse(&bytes).expect("a packet whose chain lies");
    assert_eq!(packet.upper_layer(), Err(Ipv6Error::ExtensionLength(128)));
}

#[test]
fn a_chain_that_never_ends_is_a_drop_and_not_a_loop() {
    // Every header names its own kind, so the walk never reaches an
    // upper layer. It ends at the bound, not by running round.
    let mut body = Vec::new();
    for _ in 0..64 {
        body.extend_from_slice(&short_extension(Protocol::HOP_BY_HOP));
    }
    let bytes = packet(HOST, PEER, Protocol::HOP_BY_HOP, DEFAULT_HOP_LIMIT, &body);
    let packet = Packet::parse(&bytes).expect("a chain without an end");
    assert!(matches!(
        packet.upper_layer(),
        Err(Ipv6Error::ChainTooLong { .. })
    ));
}

#[test]
fn a_chain_that_ends_in_no_next_header_carries_nothing() {
    let mut body = short_extension(Protocol::NO_NEXT_HEADER);
    body.extend_from_slice(&PAYLOAD);
    let bytes = packet(HOST, PEER, Protocol::HOP_BY_HOP, DEFAULT_HOP_LIMIT, &body);
    let packet = Packet::parse(&bytes).expect("a chain that ends");
    let upper = packet.upper_layer().expect("a chain of one");
    assert_eq!(upper.protocol, Protocol::NO_NEXT_HEADER);
    // RFC 8200, section 4.7: whatever follows is not a header, and the
    // walk hands it over without naming it.
    assert_eq!(upper.payload, &PAYLOAD);
}

#[test]
fn a_chain_that_ends_inside_a_header_is_dropped() {
    // Four bytes where a header takes at least two and then a length.
    let bytes = packet(HOST, PEER, Protocol::HOP_BY_HOP, DEFAULT_HOP_LIMIT, &[0]);
    let packet = Packet::parse(&bytes).expect("a packet with one byte of chain");
    assert!(matches!(packet.upper_layer(), Err(Ipv6Error::Wire(_))));
}

#[test]
fn a_written_header_reads_back_as_itself() {
    let header = Header::new(HOST, PEER, Protocol::TCP, PAYLOAD.len());
    assert_eq!(header.hop_limit, DEFAULT_HOP_LIMIT);
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    header.write(&mut writer).expect("room for a header");
    writer.write_bytes(&PAYLOAD).expect("room for the payload");
    let written = writer.written().to_vec();
    assert_eq!(written.len(), HEADER_LEN + PAYLOAD.len());
    assert_eq!(written[0].wrapping_shr(4), VERSION);
    let packet = Packet::parse(&written).expect("what was written");
    assert_eq!(packet.source(), HOST);
    assert_eq!(packet.destination(), PEER);
    assert_eq!(packet.next_header(), Protocol::TCP);
    assert_eq!(packet.payload(), &PAYLOAD);
}

#[test]
fn a_payload_longer_than_the_length_field_is_refused() {
    let header = Header::new(HOST, PEER, Protocol::TCP, 65536);
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        header.write(&mut writer),
        Err(Ipv6Error::PayloadTooLong(65536))
    );
    assert_eq!(writer.position(), 0);
}

#[test]
fn a_buffer_too_small_for_a_header_takes_nothing() {
    let header = Header::new(HOST, PEER, Protocol::TCP, 0);
    let mut buffer = [0u8; HEADER_LEN - 1];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(header.write(&mut writer), Err(Ipv6Error::Wire(_))));
    assert_eq!(writer.position(), 0);
}

#[test]
fn a_written_fragment_header_reads_back_as_itself() {
    let header = FragmentHeader {
        next_header: Protocol::UDP,
        offset: 1448,
        more: true,
        identification: 0x0102_0304,
    };
    let mut buffer = [0u8; 16];
    let mut writer = Writer::new(&mut buffer);
    header.write(&mut writer).expect("room");
    assert_eq!(
        writer.written(),
        fragment_header(Protocol::UDP, 1448, true, 0x0102_0304)
    );
}

#[test]
fn a_fragment_offset_is_rounded_down_to_the_eight_bytes_the_field_counts() {
    let header = FragmentHeader {
        next_header: Protocol::UDP,
        offset: 15,
        more: false,
        identification: 0,
    };
    let mut buffer = [0u8; 16];
    let mut writer = Writer::new(&mut buffer);
    header.write(&mut writer).expect("room");
    assert_eq!(
        writer.written(),
        fragment_header(Protocol::UDP, 8, false, 0)
    );
}

#[test]
fn a_buffer_too_small_for_a_fragment_header_takes_nothing() {
    let header = FragmentHeader {
        next_header: Protocol::UDP,
        offset: 0,
        more: false,
        identification: 0,
    };
    let mut buffer = [0u8; 7];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(header.write(&mut writer), Err(Ipv6Error::Wire(_))));
    assert_eq!(writer.position(), 0);
}

#[test]
fn no_byte_stream_makes_the_parser_panic_or_reach_past_itself() {
    check("ipv6_packet_parse", &bytes(0..=96), |input| {
        let Ok(packet) = Packet::parse(input) else {
            return Ok(());
        };
        if packet.total_len() > input.len() {
            return Err("a packet longer than the bytes it was read from".into());
        }
        if packet.header().len() != HEADER_LEN {
            return Err("a header that is not forty bytes".into());
        }
        let Ok(upper) = packet.upper_layer() else {
            return Ok(());
        };
        if upper.payload.len() > packet.payload().len() {
            return Err("a chain walk that grew the payload".into());
        }
        if upper.headers > MAX_EXTENSION_HEADERS || upper.bytes > MAX_EXTENSION_BYTES {
            return Err("a chain accepted past its bound".into());
        }
        Ok(())
    });
}

#[test]
fn every_accepted_packet_re_encodes_to_the_bytes_it_came_from() {
    check("ipv6_packet_round_trip", &bytes(0..=96), |input| {
        let Ok(packet) = Packet::parse(input) else {
            return Ok(());
        };
        let header = Header {
            source: packet.source(),
            destination: packet.destination(),
            next_header: packet.next_header(),
            hop_limit: packet.hop_limit(),
            payload_len: packet.payload().len(),
        };
        let mut buffer = [0u8; 256];
        let mut writer = Writer::new(&mut buffer);
        if header.write(&mut writer).is_err() {
            return Err("a header that was read and cannot be written".into());
        }
        // The traffic class and the flow label are not carried, so they
        // are the four bits and twenty-eight bits this comparison steps
        // over; everything else is a field this crate reads.
        let written = writer.written();
        if written[6..] != input[6..HEADER_LEN] {
            return Err("a header that did not write back as itself".into());
        }
        Ok(())
    });
}

#[test]
fn an_unspecified_source_is_read_as_one() {
    let bytes = packet(Ipv6Addr::UNSPECIFIED, PEER, Protocol::ICMPV6, 255, &PAYLOAD);
    let packet = Packet::parse(&bytes).expect("a packet from nowhere");
    assert!(packet.source().is_unspecified());
}
