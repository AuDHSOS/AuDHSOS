// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The IPv4 header against the field layout of RFC 791, section 3.1.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a header at known offsets"
)]

use net_wire::{Ipv4Addr, Protocol, Writer, is_valid};
use test_support::generators::bytes;
use test_support::property::check;

use crate::error::IpError;
use crate::header::{
    CHECKSUM_OFFSET, DEFAULT_TTL, Datagram, FRAGMENT_UNIT, Header, MIN_HEADER_LEN,
};

/// A datagram of twenty-eight bytes: five words of header and eight of
/// payload, from 10.0.0.2 to 10.0.0.1, carrying UDP, with the
/// don't-fragment bit set and the checksum `0x0a89` that the header sums
/// to.
const HEADER: [u8; MIN_HEADER_LEN] = [
    0x45, 0x00, 0x00, 0x1C, 0x1C, 0x46, 0x40, 0x00, 0x40, 0x11, 0x0A, 0x89, 0x0A, 0x00, 0x00, 0x02,
    0x0A, 0x00, 0x00, 0x01,
];

/// The same with one four-byte option, so six words of header and eight of
/// payload, and no flags.
const HEADER_WITH_OPTION: [u8; 24] = [
    0x46, 0x00, 0x00, 0x20, 0x1C, 0x47, 0x00, 0x00, 0x40, 0x11, 0x3E, 0x80, 0x0A, 0x00, 0x00, 0x02,
    0x0A, 0x00, 0x00, 0x01, 0x07, 0x04, 0x04, 0x00,
];

/// The eight bytes of payload the two headers declare.
const PAYLOAD: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// The datagram of `header` followed by the payload.
fn datagram_of(header: &[u8]) -> Vec<u8> {
    let mut bytes = header.to_vec();
    bytes.extend_from_slice(&PAYLOAD);
    bytes
}

#[test]
fn a_minimum_header_reads_as_the_fields_rfc_791_names() {
    let bytes = datagram_of(&HEADER);
    let datagram = Datagram::parse(&bytes).expect("a well-formed datagram");
    assert_eq!(datagram.source(), Ipv4Addr::new(10, 0, 0, 2));
    assert_eq!(datagram.destination(), Ipv4Addr::new(10, 0, 0, 1));
    assert_eq!(datagram.protocol(), Protocol::UDP);
    assert_eq!(datagram.ttl(), 64);
    assert!(!datagram.is_expired());
    assert_eq!(datagram.identification(), 0x1C46);
    assert!(datagram.dont_fragment());
    assert!(!datagram.more_fragments());
    assert_eq!(datagram.fragment_offset(), 0);
    assert!(!datagram.is_fragment());
    assert!(datagram.is_initial_fragment());
    assert_eq!(datagram.header_len(), MIN_HEADER_LEN);
    assert_eq!(datagram.header(), &HEADER);
    assert_eq!(datagram.payload(), &PAYLOAD);
    assert_eq!(datagram.total_len(), 28);
}

#[test]
fn options_are_stepped_over_and_never_read() {
    let bytes = datagram_of(&HEADER_WITH_OPTION);
    let datagram = Datagram::parse(&bytes).expect("a header with an option");
    assert_eq!(datagram.header_len(), 24);
    assert_eq!(datagram.payload(), &PAYLOAD);
    // The option is inside the header and outside the payload, which is
    // the whole of what this crate does with it.
    assert_eq!(
        &datagram.header()[MIN_HEADER_LEN..],
        &[0x07, 0x04, 0x04, 0x00]
    );
}

#[test]
fn a_version_that_is_not_four_is_refused() {
    let mut bytes = datagram_of(&HEADER);
    bytes[0] = 0x65;
    assert_eq!(Datagram::parse(&bytes), Err(IpError::Version(6)));
    bytes[0] = 0x05;
    assert_eq!(Datagram::parse(&bytes), Err(IpError::Version(0)));
}

#[test]
fn a_header_length_below_five_words_or_beyond_the_buffer_is_refused() {
    let mut bytes = datagram_of(&HEADER);
    for (words, expected) in [(0u8, 0u8), (4, 4)] {
        bytes[0] = 0x40 | words;
        assert_eq!(
            Datagram::parse(&bytes),
            Err(IpError::HeaderLength(expected))
        );
    }
    // Fifteen words is sixty bytes, which this datagram does not have.
    bytes[0] = 0x4F;
    assert_eq!(Datagram::parse(&bytes), Err(IpError::HeaderLength(15)));
}

#[test]
fn a_total_length_below_the_header_or_beyond_the_frame_is_refused() {
    let mut bytes = datagram_of(&HEADER);
    bytes[2] = 0x00;
    bytes[3] = 0x13; // nineteen, one below the header
    assert_eq!(Datagram::parse(&bytes), Err(IpError::TotalLength(19)));
    bytes[3] = 0xFF; // 255, beyond the twenty-eight bytes there are
    assert_eq!(Datagram::parse(&bytes), Err(IpError::TotalLength(255)));
}

#[test]
fn a_total_length_shorter_than_the_frame_cuts_the_payload() {
    let mut bytes = datagram_of(&HEADER);
    bytes.extend_from_slice(&[0xFF; 4]);
    // The length still says twenty-eight, so the four bytes appended
    // after it are not part of the datagram.
    let datagram = Datagram::parse(&bytes).expect("a datagram inside a longer buffer");
    assert_eq!(datagram.payload(), &PAYLOAD);
    assert_eq!(datagram.total_len(), 28);
}

#[test]
fn a_header_whose_checksum_does_not_verify_is_refused() {
    let mut bytes = datagram_of(&HEADER);
    bytes[8] = 0x3F; // one less hop, and the checksum no longer fits
    assert_eq!(Datagram::parse(&bytes), Err(IpError::Checksum(0x0A89)));
}

#[test]
fn a_buffer_that_ends_inside_the_header_is_refused() {
    let bytes = datagram_of(&HEADER);
    for length in 0..MIN_HEADER_LEN {
        assert!(
            Datagram::parse(&bytes[..length]).is_err(),
            "{length} bytes parsed"
        );
    }
}

#[test]
fn the_fragment_fields_are_read_in_the_units_rfc_791_counts_them_in() {
    let mut bytes = datagram_of(&HEADER);
    // More fragments, offset 185 units, which is 1480 bytes.
    bytes[6] = 0x20;
    bytes[7] = 0xB9;
    fix_checksum(&mut bytes);
    let datagram = Datagram::parse(&bytes).expect("a fragment");
    assert!(datagram.more_fragments());
    assert!(!datagram.dont_fragment());
    assert_eq!(datagram.fragment_offset(), 185 * FRAGMENT_UNIT);
    assert!(datagram.is_fragment());
    assert!(!datagram.is_initial_fragment());
}

#[test]
fn a_time_to_live_of_zero_is_reported_and_not_refused() {
    let mut bytes = datagram_of(&HEADER);
    bytes[8] = 0;
    fix_checksum(&mut bytes);
    // The datagram parses: this host does not forward, so an expired
    // datagram addressed to it is not its problem to refuse.
    let datagram = Datagram::parse(&bytes).expect("a datagram with no hops left");
    assert!(datagram.is_expired());
    assert_eq!(datagram.ttl(), 0);
}

#[test]
fn a_written_header_verifies_and_reads_back() {
    let header = Header::new(
        Ipv4Addr::new(10, 0, 0, 2),
        Ipv4Addr::new(10, 0, 0, 1),
        Protocol::UDP,
        PAYLOAD.len(),
    );
    assert_eq!(header.ttl, DEFAULT_TTL);
    let mut buffer = [0u8; 28];
    let mut writer = Writer::new(&mut buffer);
    header.write(&mut writer).expect("room for a header");
    writer.write_bytes(&PAYLOAD).expect("room for the payload");

    assert!(is_valid(&buffer[..MIN_HEADER_LEN]));
    let datagram = Datagram::parse(&buffer).expect("what was just written");
    assert_eq!(datagram.source(), Ipv4Addr::new(10, 0, 0, 2));
    assert_eq!(datagram.destination(), Ipv4Addr::new(10, 0, 0, 1));
    assert_eq!(datagram.protocol(), Protocol::UDP);
    assert_eq!(datagram.payload(), &PAYLOAD);
    assert!(!datagram.dont_fragment());
    assert_eq!(datagram.header_len(), MIN_HEADER_LEN);
}

#[test]
fn the_flags_and_the_offset_are_written_where_they_are_read_from() {
    let mut header = Header::new(
        Ipv4Addr::new(10, 0, 0, 2),
        Ipv4Addr::new(10, 0, 0, 1),
        Protocol::TCP,
        0,
    );
    header.dont_fragment = true;
    header.more_fragments = true;
    header.fragment_offset = 1480;
    header.identification = 0x1C46;
    header.ttl = 1;
    let mut buffer = [0u8; MIN_HEADER_LEN];
    let mut writer = Writer::new(&mut buffer);
    header.write(&mut writer).expect("room for a header");

    let datagram = Datagram::parse(&buffer).expect("what was just written");
    assert!(datagram.dont_fragment());
    assert!(datagram.more_fragments());
    assert_eq!(datagram.fragment_offset(), 1480);
    assert_eq!(datagram.identification(), 0x1C46);
    assert_eq!(datagram.ttl(), 1);
    assert_eq!(datagram.protocol(), Protocol::TCP);
    assert_eq!(buffer[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 2], buffer[10..12]);
}

#[test]
fn a_header_that_does_not_fit_writes_nothing() {
    let header = Header::new(Ipv4Addr::LOCALHOST, Ipv4Addr::LOCALHOST, Protocol::UDP, 0);
    let mut buffer = [0u8; MIN_HEADER_LEN - 1];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(header.write(&mut writer), Err(IpError::Wire(_))));
    assert_eq!(writer.position(), 0);
    assert_eq!(buffer, [0u8; MIN_HEADER_LEN - 1]);
}

#[test]
fn a_payload_beyond_the_length_field_is_refused() {
    let header = Header::new(
        Ipv4Addr::LOCALHOST,
        Ipv4Addr::LOCALHOST,
        Protocol::UDP,
        usize::from(u16::MAX),
    );
    let mut buffer = [0u8; MIN_HEADER_LEN];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        header.write(&mut writer),
        Err(IpError::TotalLength(u16::MAX))
    );
    assert_eq!(writer.position(), 0);
}

#[test]
fn what_is_written_parses_back_to_what_it_was_written_from() {
    check("header round trip", &bytes(0..=200), |payload| {
        let header = Header::new(
            Ipv4Addr::new(10, 0, 0, 2),
            Ipv4Addr::new(10, 0, 0, 1),
            Protocol::UDP,
            payload.len(),
        );
        let mut buffer = vec![0u8; MIN_HEADER_LEN + payload.len()];
        let mut writer = Writer::new(&mut buffer);
        header
            .write(&mut writer)
            .map_err(|error| error.to_string())?;
        writer
            .write_bytes(payload)
            .map_err(|error| error.to_string())?;
        let datagram = Datagram::parse(&buffer).map_err(|error| error.to_string())?;
        if datagram.payload() == payload.as_slice() && datagram.total_len() == buffer.len() {
            Ok(())
        } else {
            Err(format!("{} bytes came back changed", payload.len()))
        }
    });
}

#[test]
fn no_sequence_of_bytes_makes_the_parser_do_anything_but_answer() {
    check(
        "header parse is total",
        &bytes(0..=64),
        |input| match Datagram::parse(input) {
            Ok(datagram) => {
                if datagram.total_len() <= input.len() {
                    Ok(())
                } else {
                    Err("a datagram reached past its buffer".to_owned())
                }
            }
            Err(_) => Ok(()),
        },
    );
}

/// Puts the header checksum right after a test has changed a field.
fn fix_checksum(bytes: &mut [u8]) {
    bytes[CHECKSUM_OFFSET] = 0;
    bytes[CHECKSUM_OFFSET + 1] = 0;
    let sum = net_wire::checksum(&bytes[..MIN_HEADER_LEN]);
    bytes[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 2].copy_from_slice(&sum.to_be_bytes());
}
