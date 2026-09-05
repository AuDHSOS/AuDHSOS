// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Cutting a datagram up and putting one back together, per RFC 791,
//! section 3.2.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a datagram at known offsets"
)]

use audhsos_time::{Duration, Instant};
use net_wire::{Ipv4Addr, Protocol, Writer};

use crate::error::IpError;
use crate::fragment::{Assembled, Fragments, REASSEMBLY_TIMEOUT, Reassembler, fragment};
use crate::header::{Datagram, Header, MIN_HEADER_LEN};

/// The MTU of an Ethernet, which is what the interface below this one
/// carries.
const MTU: usize = 1500;

/// How much payload one fragment holds at that MTU: 1480, which is a
/// multiple of eight.
const PER_FRAGMENT: usize = MTU - MIN_HEADER_LEN;

/// Four buffers of four kibibytes each, which is room for three
/// fragments of an Ethernet MTU.
type Buffers = Reassembler<4, 4096>;

/// The header every datagram here is written from.
fn header(payload_len: usize) -> Header {
    let mut header = Header::new(
        Ipv4Addr::new(192, 168, 1, 7),
        Ipv4Addr::new(192, 168, 1, 1),
        Protocol::UDP,
        payload_len,
    );
    header.identification = 0x1C46;
    header
}

/// A payload whose every byte says where it belongs.
fn payload_of(length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| u8::try_from(index % 251).unwrap_or(0))
        .collect()
}

/// The pieces `payload` is cut into for `mtu`.
fn pieces(header: &Header, payload: &[u8], mtu: usize) -> Result<Vec<Vec<u8>>, IpError> {
    let mut out = Vec::new();
    let mut buffer = vec![0u8; mtu];
    fragment(header, payload, mtu, &mut buffer, |piece| {
        out.push(piece.to_vec());
        Ok(())
    })?;
    Ok(out)
}

#[test]
fn a_datagram_at_the_mtu_is_not_cut_up() {
    let payload = payload_of(PER_FRAGMENT);
    let out = pieces(&header(payload.len()), &payload, MTU).expect("it fits");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].len(), MTU);
    let datagram = Datagram::parse(&out[0]).expect("a datagram");
    assert!(!datagram.is_fragment());
    assert!(!datagram.more_fragments());
    assert_eq!(datagram.payload(), payload.as_slice());
}

#[test]
fn one_byte_more_makes_two_pieces_that_reassemble_to_the_original() {
    let payload = payload_of(PER_FRAGMENT + 1);
    let out = pieces(&header(payload.len()), &payload, MTU).expect("it is cut up");
    assert_eq!(out.len(), 2);

    let first = Datagram::parse(&out[0]).expect("the first piece");
    assert!(first.more_fragments());
    assert_eq!(first.fragment_offset(), 0);
    assert_eq!(first.payload().len(), PER_FRAGMENT);
    let second = Datagram::parse(&out[1]).expect("the second piece");
    assert!(!second.more_fragments());
    assert_eq!(second.fragment_offset(), PER_FRAGMENT);
    assert_eq!(second.payload().len(), 1);

    let mut buffers = Buffers::new();
    assert!(
        buffers
            .accept(first, Instant::ZERO)
            .expect("a fragment")
            .is_none()
    );
    let assembled = buffers
        .accept(second, Instant::ZERO)
        .expect("a fragment")
        .expect("the last piece completes it");
    assert_eq!(assembled.payload(), payload.as_slice());
}

#[test]
fn the_dont_fragment_bit_turns_an_oversized_datagram_into_an_error() {
    let payload = payload_of(PER_FRAGMENT + 1);
    let mut header = header(payload.len());
    header.dont_fragment = true;
    assert_eq!(
        pieces(&header, &payload, MTU),
        Err(IpError::WouldFragment {
            length: payload.len(),
            mtu: MTU
        })
    );
    // And a datagram that fits goes out whole with the bit set.
    let payload = payload_of(PER_FRAGMENT);
    let out = pieces(&header, &payload, MTU).expect("it fits");
    assert_eq!(out.len(), 1);
    assert!(
        Datagram::parse(&out[0])
            .expect("a datagram")
            .dont_fragment()
    );
}

#[test]
fn an_empty_payload_still_makes_one_datagram() {
    let out = pieces(&header(0), &[], MTU).expect("a datagram with no payload");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].len(), MIN_HEADER_LEN);
}

#[test]
fn an_mtu_with_no_room_for_a_fragment_is_refused() {
    for mtu in [0, MIN_HEADER_LEN, MIN_HEADER_LEN + 7] {
        assert_eq!(
            Fragments::new(100, mtu),
            Err(IpError::WouldFragment { length: 100, mtu })
        );
    }
    // Eight bytes behind the header is the smallest that works.
    let pieces = Fragments::new(100, MIN_HEADER_LEN + 8).expect("eight bytes a piece");
    assert_eq!(pieces.count(), 13);
    assert!(!pieces.is_whole());
}

#[test]
fn the_pieces_are_counted_before_they_are_walked() {
    let whole = Fragments::new(PER_FRAGMENT, MTU).expect("it fits");
    assert_eq!(whole.count(), 1);
    assert!(whole.is_whole());
    let two = Fragments::new(PER_FRAGMENT + 1, MTU).expect("two pieces");
    assert_eq!(two.count(), 2);
    assert!(!two.is_whole());
    let empty = Fragments::new(0, MTU).expect("one empty piece");
    assert_eq!(empty.count(), 1);
    assert!(empty.is_whole());
}

#[test]
fn fragments_arrive_in_order_in_reverse_and_with_a_duplicate() {
    let payload = payload_of(PER_FRAGMENT * 2 + 5);
    let out = pieces(&header(payload.len()), &payload, MTU).expect("three pieces");
    assert_eq!(out.len(), 3);

    for order in [[0usize, 1, 2], [2, 1, 0], [1, 2, 0]] {
        let mut buffers = Buffers::new();
        let mut done = None;
        for index in order {
            let piece = Datagram::parse(&out[index]).expect("a piece");
            if let Some(assembled) = buffers.accept(piece, Instant::ZERO).expect("a fragment") {
                done = Some(assembled.payload().to_vec());
            }
        }
        assert_eq!(done.as_deref(), Some(payload.as_slice()), "{order:?}");
    }

    // A duplicate of a piece already held changes nothing.
    let mut buffers = Buffers::new();
    for index in [0usize, 0, 1, 2] {
        let piece = Datagram::parse(&out[index]).expect("a piece");
        let _ = buffers.accept(piece, Instant::ZERO).expect("a fragment");
    }
    let last = Datagram::parse(&out[2]).expect("a piece");
    let mut fresh = Buffers::new();
    for index in [0usize, 0, 1] {
        let piece = Datagram::parse(&out[index]).expect("a piece");
        assert!(
            fresh
                .accept(piece, Instant::ZERO)
                .expect("a fragment")
                .is_none()
        );
    }
    let assembled = fresh
        .accept(last, Instant::ZERO)
        .expect("a fragment")
        .expect("the last piece completes it");
    assert_eq!(assembled.payload(), payload.as_slice());
}

#[test]
fn an_overlapping_fragment_discards_the_whole_datagram() {
    let payload = payload_of(PER_FRAGMENT + 8);
    let out = pieces(&header(payload.len()), &payload, MTU).expect("two pieces");

    // A second piece that claims bytes the first already carried, with
    // different content.
    let mut evil = vec![0u8; MIN_HEADER_LEN + 16];
    let mut evil_header = header(16);
    evil_header.fragment_offset = PER_FRAGMENT - 8;
    evil_header.more_fragments = false;
    let mut writer = Writer::new(&mut evil);
    evil_header.write(&mut writer).expect("room");
    writer.write_bytes(&[0xFF; 16]).expect("room");

    let mut buffers = Buffers::new();
    let first = Datagram::parse(&out[0]).expect("the first piece");
    assert!(
        buffers
            .accept(first, Instant::ZERO)
            .expect("a fragment")
            .is_none()
    );
    let overlapping = Datagram::parse(&evil).expect("a fragment");
    assert_eq!(
        buffers.accept(overlapping, Instant::ZERO).map(|_| ()),
        Err(IpError::OverlappingFragment)
    );
}

#[test]
fn a_missing_fragment_expires_at_the_deadline_and_frees_its_buffer() {
    let payload = payload_of(PER_FRAGMENT + 1);
    let out = pieces(&header(payload.len()), &payload, MTU).expect("two pieces");
    let mut buffers = Buffers::new();
    let first = Datagram::parse(&out[0]).expect("the first piece");
    assert!(
        buffers
            .accept(first, Instant::ZERO)
            .expect("a fragment")
            .is_none()
    );
    assert_eq!(buffers.len(), 1);
    let deadline = Instant::ZERO.saturating_add(REASSEMBLY_TIMEOUT);
    assert_eq!(buffers.poll_at(), Some(deadline));

    // One microsecond before, it is still there.
    assert_eq!(
        buffers.poll(Instant::from_micros(deadline.as_micros() - 1)),
        0
    );
    assert_eq!(buffers.len(), 1);

    assert_eq!(buffers.poll(deadline), 1);
    assert!(buffers.is_empty());
    assert_eq!(buffers.poll_at(), None);
}

#[test]
fn more_concurrent_datagrams_than_buffers_evicts_the_oldest() {
    let payload = payload_of(PER_FRAGMENT + 1);
    let mut buffers = Reassembler::<2, 2048>::new();
    let mut kept = Vec::new();
    for (index, identification) in [1u16, 2, 3].into_iter().enumerate() {
        let mut header = header(payload.len());
        header.identification = identification;
        let out = pieces(&header, &payload, MTU).expect("two pieces");
        kept.push(out);
        let first = Datagram::parse(&kept[index][0]).expect("the first piece");
        let now = Instant::from_micros(u64::try_from(index).unwrap_or(0));
        assert!(buffers.accept(first, now).expect("a fragment").is_none());
    }
    // Two slots, three datagrams: the first one is gone.
    assert_eq!(buffers.len(), 2);
    let second_of_first = Datagram::parse(&kept[0][1]).expect("the second piece");
    // Its other piece starts a fresh datagram rather than completing one,
    // so nothing comes out.
    assert!(
        buffers
            .accept(second_of_first, Instant::from_micros(10))
            .expect("a fragment")
            .is_none()
    );
}

#[test]
fn a_datagram_that_was_never_fragmented_is_not_copied() {
    let payload = payload_of(100);
    let out = pieces(&header(payload.len()), &payload, MTU).expect("one piece");
    let datagram = Datagram::parse(&out[0]).expect("a datagram");
    let mut buffers = Buffers::new();
    let assembled = buffers
        .accept(datagram, Instant::ZERO)
        .expect("a whole datagram")
        .expect("it needs no reassembly");
    assert!(matches!(assembled, Assembled::Whole(_)));
    assert_eq!(assembled.payload(), payload.as_slice());
    assert!(buffers.is_empty());
}

#[test]
fn a_fragment_beyond_the_buffer_is_refused_before_a_byte_is_copied() {
    let mut bytes = vec![0u8; MIN_HEADER_LEN + 8];
    let mut header = header(8);
    header.fragment_offset = 8192;
    header.more_fragments = true;
    let mut writer = Writer::new(&mut bytes);
    header.write(&mut writer).expect("room");
    writer.write_bytes(&[1u8; 8]).expect("room");

    let mut buffers = Buffers::new();
    let far = Datagram::parse(&bytes).expect("a fragment");
    assert_eq!(
        buffers.accept(far, Instant::ZERO).map(|_| ()),
        Err(IpError::TooLarge(8200))
    );
    assert!(buffers.is_empty());
}

#[test]
fn a_reassembled_datagram_can_be_released_by_hand() {
    let payload = payload_of(PER_FRAGMENT + 1);
    let out = pieces(&header(payload.len()), &payload, MTU).expect("two pieces");
    let mut buffers = Buffers::new();
    let first = Datagram::parse(&out[0]).expect("the first piece");
    let _ = buffers.accept(first, Instant::ZERO).expect("a fragment");
    assert_eq!(buffers.len(), 1);
    buffers.release(first);
    assert!(buffers.is_empty());
    // Releasing what is not there does nothing.
    buffers.release(first);
    assert!(buffers.is_empty());
}

#[test]
fn a_timeout_of_the_callers_own_is_used_as_given() {
    let payload = payload_of(PER_FRAGMENT + 1);
    let out = pieces(&header(payload.len()), &payload, MTU).expect("two pieces");
    let timeout = Duration::from_millis(500);
    let mut buffers = Reassembler::<2, 2048>::with_timeout(timeout);
    let first = Datagram::parse(&out[0]).expect("the first piece");
    let _ = buffers.accept(first, Instant::ZERO).expect("a fragment");
    assert_eq!(
        buffers.poll_at(),
        Some(Instant::ZERO.saturating_add(timeout))
    );
    assert_eq!(buffers.poll(Instant::ZERO.saturating_add(timeout)), 1);
    assert!(buffers.is_empty());
    assert_eq!(Reassembler::<2, 2048>::default().len(), 0);
}
