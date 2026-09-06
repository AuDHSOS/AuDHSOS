// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The segment format of RFC 9293, section 3.1, and its option list.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a segment at known offsets"
)]

use net_wire::{IpAddr, Ipv4Addr, Ipv6Addr, Port, Protocol, Writer, transport};
use test_support::generators::bytes;
use test_support::property::check;

use crate::error::TcpError;
use crate::segment::{Flags, HEADER_LEN, Segment, reset_for};
use crate::seq::SeqNumber;

/// This host.
const HERE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));

/// The other host.
const PEER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

/// This host, over IPv6.
const HERE_V6: IpAddr = IpAddr::V6(Ipv6Addr::from_octets([
    0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01,
]));

/// The other host, over IPv6.
const PEER_V6: IpAddr = IpAddr::V6(Ipv6Addr::from_octets([
    0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x02,
]));

/// An acknowledgment from port 1024 to port 80, sequence 1, window 8192,
/// between `192.168.1.10` and `192.168.1.20`, with the checksum summed by
/// hand: the pseudo-header words `c0a8 010a c0a8 0114 0006 0014` and the
/// header words `0400 0050 0000 0001 0000 0000 5010 2000 0000 0000` add
/// up with the end-around carry to `f7ea`, whose complement is `0815`.
const VECTOR: [u8; 20] = [
    0x04, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x50, 0x10, 0x20, 0x00,
    0x08, 0x15, 0x00, 0x00,
];

/// Writes a segment and answers how long it is.
fn write(segment: &Segment<'_>, source: IpAddr, destination: IpAddr, buffer: &mut [u8]) -> usize {
    let mut writer = Writer::new(buffer);
    segment
        .write(&mut writer, source, destination)
        .expect("room for the segment");
    writer.position()
}

/// The acknowledgment the vector holds.
fn vector_segment() -> Segment<'static> {
    let mut segment = Segment::new(Port::new(1024), Port::new(80), Flags::ACK);
    segment.seq = SeqNumber::new(1);
    segment.window = 8192;
    segment
}

#[test]
fn the_fields_sit_where_rfc_9293_puts_them() {
    let segment = Segment::parse(&VECTOR, HERE, PEER).expect("a segment");
    assert_eq!(segment.source_port, Port::new(1024));
    assert_eq!(segment.destination_port, Port::new(80));
    assert_eq!(segment.seq, SeqNumber::new(1));
    assert_eq!(segment.ack, SeqNumber::new(0));
    assert_eq!(segment.flags, Flags::ACK);
    assert_eq!(segment.window, 8192);
    assert_eq!(segment.max_segment_size, None);
    assert!(segment.payload.is_empty());

    let mut buffer = [0u8; 40];
    let len = write(&vector_segment(), HERE, PEER, &mut buffer);
    assert_eq!(&buffer[..len], &VECTOR);
}

#[test]
fn a_syn_announces_a_maximum_segment_size_and_reads_it_back() {
    let mut segment = Segment::new(Port::new(1024), Port::new(80), Flags::SYN);
    segment.seq = SeqNumber::new(7);
    segment.max_segment_size = Some(1460);
    let mut buffer = [0u8; 40];
    let len = write(&segment, HERE, PEER, &mut buffer);
    assert_eq!(len, HEADER_LEN + 4);
    // Data offset six words, and the option itself: kind 2, length 4.
    assert_eq!(buffer[12] >> 4, 6);
    assert_eq!(&buffer[20..24], &[2, 4, 0x05, 0xB4]);
    let back = Segment::parse(&buffer[..len], HERE, PEER).expect("a segment");
    assert_eq!(back.max_segment_size, Some(1460));
    assert_eq!(back.sequence_len(), 1);
}

#[test]
fn a_segment_without_syn_carries_no_announced_size_however_the_option_reads() {
    let mut buffer = [0u8; 40];
    let mut segment = vector_segment();
    segment.max_segment_size = Some(1460);
    let len = write(&segment, HERE, PEER, &mut buffer);
    // The option is on the wire, and a segment without `SYN` may not
    // announce one, so the value is not read back.
    assert_eq!(&buffer[20..24], &[2, 4, 0x05, 0xB4]);
    let back = Segment::parse(&buffer[..len], HERE, PEER).expect("a segment");
    assert_eq!(back.max_segment_size, None);
}

#[test]
fn an_unknown_option_is_stepped_over_and_a_malformed_list_is_refused() {
    let mut segment = Segment::new(Port::new(1024), Port::new(80), Flags::SYN);
    segment.max_segment_size = Some(1460);
    let mut buffer = [0u8; 44];
    let len = write(&segment, HERE, PEER, &mut buffer);
    // Turn the option into an unknown kind of the same length, with a
    // no-operation in front of it. The list is still whole.
    buffer[20] = 1;
    buffer[21] = 30;
    buffer[22] = 3;
    buffer[23] = 0;
    let checksum = transport(HERE, PEER, Protocol::TCP, &buffer[..len]).expect("a sum");
    assert_ne!(checksum, 0);
    resum(&mut buffer[..len]);
    let back = Segment::parse(&buffer[..len], HERE, PEER).expect("a segment");
    assert_eq!(back.max_segment_size, None);

    // A length below the two bytes an option takes is not a whole option.
    buffer[22] = 1;
    resum(&mut buffer[..len]);
    assert_eq!(
        Segment::parse(&buffer[..len], HERE, PEER),
        Err(TcpError::BadOption(30))
    );

    // Neither is one that reaches past the header.
    buffer[22] = 8;
    resum(&mut buffer[..len]);
    assert_eq!(
        Segment::parse(&buffer[..len], HERE, PEER),
        Err(TcpError::BadOption(30))
    );
}

#[test]
fn the_end_of_the_list_stops_the_walk() {
    let mut segment = Segment::new(Port::new(1024), Port::new(80), Flags::SYN);
    segment.max_segment_size = Some(1460);
    let mut buffer = [0u8; 44];
    let len = write(&segment, HERE, PEER, &mut buffer);
    // End of list, then bytes that are not an option at all.
    buffer[20] = 0;
    buffer[21] = 99;
    buffer[22] = 1;
    buffer[23] = 0;
    resum(&mut buffer[..len]);
    let back = Segment::parse(&buffer[..len], HERE, PEER).expect("a segment");
    assert_eq!(back.max_segment_size, None);
}

#[test]
fn fewer_bytes_than_a_header_are_no_segment() {
    let short = [0u8; 19];
    assert_eq!(Segment::parse(&short, HERE, PEER), Err(TcpError::Short(19)));
}

#[test]
fn a_data_offset_that_names_no_header_this_segment_has_is_refused() {
    let mut buffer = VECTOR;
    // Four words is below the five a header takes.
    buffer[12] = 0x40;
    assert_eq!(
        Segment::parse(&buffer, HERE, PEER),
        Err(TcpError::DataOffset(4))
    );
    // Six words is more than arrived.
    buffer[12] = 0x60;
    assert_eq!(
        Segment::parse(&buffer, HERE, PEER),
        Err(TcpError::DataOffset(6))
    );
}

#[test]
fn a_wrong_checksum_is_refused_in_either_family() {
    for (source, destination) in [(HERE, PEER), (HERE_V6, PEER_V6)] {
        let mut buffer = [0u8; 40];
        let len = write(&vector_segment(), source, destination, &mut buffer);
        assert_eq!(
            transport(source, destination, Protocol::TCP, &buffer[..len]),
            Ok(0)
        );
        buffer[17] ^= 0x01;
        let carried = u16::from_be_bytes([buffer[16], buffer[17]]);
        assert_eq!(
            Segment::parse(&buffer[..len], source, destination),
            Err(TcpError::Checksum(carried))
        );
    }
}

#[test]
fn two_addresses_of_different_families_have_no_pseudo_header() {
    assert_eq!(
        Segment::parse(&VECTOR, HERE, PEER_V6),
        Err(TcpError::MixedFamilies)
    );
    let mut buffer = [0u8; 40];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(
        vector_segment().write(&mut writer, HERE_V6, PEER),
        Err(TcpError::MixedFamilies)
    );
}

#[test]
fn a_buffer_without_room_takes_nothing_at_all() {
    let mut buffer = [0u8; 19];
    let mut writer = Writer::new(&mut buffer);
    assert!(vector_segment().write(&mut writer, HERE, PEER).is_err());
    assert_eq!(writer.position(), 0);
    assert_eq!(buffer, [0u8; 19]);
}

#[test]
fn syn_and_fin_each_take_a_sequence_number_of_their_own() {
    let mut segment = Segment::new(Port::new(1), Port::new(2), Flags::SYN.with(Flags::FIN));
    segment.seq = SeqNumber::new(100);
    segment.payload = b"four";
    assert_eq!(segment.sequence_len(), 6);
    assert_eq!(segment.sequence_end(), SeqNumber::new(106));

    segment.flags = Flags::ACK;
    assert_eq!(segment.sequence_len(), 4);
    assert_eq!(segment.sequence_end(), SeqNumber::new(104));
}

#[test]
fn the_control_bits_are_a_set_and_read_back_as_letters() {
    let flags = Flags::SYN.with(Flags::ACK);
    assert!(flags.has(Flags::SYN));
    assert!(flags.has(Flags::ACK));
    assert!(!flags.has(Flags::FIN));
    assert!(flags.has_any(Flags::FIN.with(Flags::SYN)));
    assert!(!flags.has_any(Flags::RST.with(Flags::URG)));
    assert_eq!(flags.without(Flags::ACK), Flags::SYN);
    assert_eq!(flags.bits(), 0x12);
    assert_eq!(Flags::new(0x12), flags);
    assert_eq!(flags.to_string(), ".A..S.");
    assert_eq!(Flags::NONE.to_string(), "......");
    assert_eq!(
        Flags::URG
            .with(Flags::PSH)
            .with(Flags::RST)
            .with(Flags::FIN)
            .to_string(),
        "U.PR.F"
    );
    assert_eq!(Flags::default(), Flags::NONE);
}

#[test]
fn a_reset_answers_what_the_segment_acknowledged_or_everything_it_occupied() {
    // With an acknowledgment, the reset carries that number and no more.
    let mut acknowledging = vector_segment();
    acknowledging.ack = SeqNumber::new(500);
    let reset = reset_for(&acknowledging).expect("a reset");
    assert_eq!(reset.flags, Flags::RST);
    assert_eq!(reset.seq, SeqNumber::new(500));
    assert_eq!(reset.source_port, Port::new(80));
    assert_eq!(reset.destination_port, Port::new(1024));

    // Without one there is no such number, so it acknowledges everything
    // the segment occupied instead.
    let mut opening = Segment::new(Port::new(1024), Port::new(80), Flags::SYN);
    opening.seq = SeqNumber::new(99);
    let reset = reset_for(&opening).expect("a reset");
    assert_eq!(reset.flags, Flags::RST.with(Flags::ACK));
    assert_eq!(reset.seq, SeqNumber::new(0));
    assert_eq!(reset.ack, SeqNumber::new(100));

    // A reset is never answered with another.
    let refusal = Segment::new(Port::new(1), Port::new(2), Flags::RST);
    assert_eq!(reset_for(&refusal), None);
}

#[test]
fn no_byte_stream_makes_the_parser_panic() {
    check("tcp parse", &bytes(0..=80), |stream| {
        for (source, destination) in [(HERE, PEER), (HERE_V6, PEER_V6)] {
            if let Ok(segment) = Segment::parse(stream, source, destination)
                && segment.payload.len() > stream.len()
            {
                return Err(format!(
                    "{} bytes read out of {}",
                    segment.payload.len(),
                    stream.len()
                ));
            }
        }
        Ok(())
    });
}

#[test]
fn every_segment_that_was_written_reads_back_and_writes_the_same_bytes() {
    check("tcp round trip", &bytes(0..=48), |payload| {
        for (source, destination) in [(HERE, PEER), (HERE_V6, PEER_V6)] {
            let mut segment = vector_segment();
            segment.payload = payload;
            let mut first = [0u8; 96];
            let len = write(&segment, source, destination, &mut first);
            let back = match Segment::parse(&first[..len], source, destination) {
                Ok(back) => back,
                Err(error) => return Err(format!("{error}")),
            };
            if back.payload != payload.as_slice() {
                return Err(format!("{payload:?} came back as {:?}", back.payload));
            }
            let mut second = [0u8; 96];
            let again = write(&back, source, destination, &mut second);
            if first[..len] != second[..again] {
                return Err(format!(
                    "{:?} rewrote as {:?}",
                    &first[..len],
                    &second[..again]
                ));
            }
        }
        Ok(())
    });
}

/// Writes the checksum of `segment` back into it, after a test has changed
/// a byte.
fn resum(segment: &mut [u8]) {
    segment[16] = 0;
    segment[17] = 0;
    let sum = transport(HERE, PEER, Protocol::TCP, segment).expect("a sum");
    segment[16..18].copy_from_slice(&sum.to_be_bytes());
}
