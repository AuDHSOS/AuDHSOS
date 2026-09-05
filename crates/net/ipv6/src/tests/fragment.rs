// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reassembly of RFC 8200, section 4.5, through the buffers of `net-ip`.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a packet at known offsets"
)]

use net_ip::{IpError, Piece, Reassembler};
use net_wire::{IpAddr, Protocol};

use super::{HOST, PEER, at, fragment_header, packet};
use crate::error::Ipv6Error;
use crate::fragment::{piece_of, reassemble, release};
use crate::header::{DEFAULT_HOP_LIMIT, Packet};

/// Two datagrams of one kibibyte each.
type Buffers = Reassembler<2, 1024>;

/// The bytes a whole datagram carries here.
const DATAGRAM: [u8; 32] = [7; 32];

/// A packet carrying one piece of `DATAGRAM`.
fn piece(offset: usize, more: bool, identification: u32, length: usize) -> Vec<u8> {
    let mut body = fragment_header(Protocol::UDP, offset, more, identification);
    body.extend_from_slice(&DATAGRAM[offset..offset + length]);
    packet(HOST, PEER, Protocol::FRAGMENT, DEFAULT_HOP_LIMIT, &body)
}

/// Hands one piece to the reassembler.
fn accept<'a>(
    buffers: &'a mut Buffers,
    bytes: &'a [u8],
    micros: u64,
) -> Result<Option<&'a [u8]>, Ipv6Error> {
    let packet = Packet::parse(bytes)?;
    let upper = packet.upper_layer()?;
    reassemble(buffers, packet, upper, at(micros))
}

#[test]
fn a_packet_that_is_not_a_fragment_is_not_copied() {
    let bytes = packet(HOST, PEER, Protocol::UDP, DEFAULT_HOP_LIMIT, &DATAGRAM);
    let mut buffers = Buffers::new();
    let payload = accept(&mut buffers, &bytes, 0)
        .expect("no reassembly needed")
        .expect("the whole datagram");
    assert_eq!(payload, &DATAGRAM);
    assert!(buffers.is_empty(), "nothing was buffered");
}

#[test]
fn an_atomic_fragment_is_a_whole_datagram() {
    // A fragment header at offset zero with no more behind it names a
    // whole datagram (RFC 8200, section 4.5), and holding a buffer for
    // one is what a sender of them would be asking for.
    let bytes = piece(0, false, 1, DATAGRAM.len());
    let mut buffers = Buffers::new();
    let payload = accept(&mut buffers, &bytes, 0)
        .expect("no reassembly needed")
        .expect("the whole datagram");
    assert_eq!(payload, &DATAGRAM);
    assert!(buffers.is_empty());
}

#[test]
fn two_pieces_are_put_back_together() {
    let first = piece(0, true, 1, 16);
    let second = piece(16, false, 1, 16);
    let mut buffers = Buffers::new();
    assert_eq!(accept(&mut buffers, &first, 0), Ok(None));
    assert_eq!(buffers.len(), 1);
    let payload = accept(&mut buffers, &second, 1)
        .expect("the second piece completes it")
        .expect("a whole datagram");
    assert_eq!(payload, &DATAGRAM);
}

#[test]
fn pieces_that_arrive_out_of_order_are_still_put_back_together() {
    let first = piece(0, true, 1, 16);
    let second = piece(16, false, 1, 16);
    let mut buffers = Buffers::new();
    assert_eq!(accept(&mut buffers, &second, 0), Ok(None));
    let payload = accept(&mut buffers, &first, 1)
        .expect("the first piece completes it")
        .expect("a whole datagram");
    assert_eq!(payload, &DATAGRAM);
}

#[test]
fn overlapping_fragments_discard_the_whole_datagram() {
    let first = piece(0, true, 1, 16);
    // A second piece claiming the same bytes with different contents.
    let mut body = fragment_header(Protocol::UDP, 8, true, 1);
    body.extend_from_slice(&[9; 16]);
    let overlapping = packet(HOST, PEER, Protocol::FRAGMENT, DEFAULT_HOP_LIMIT, &body);

    let mut buffers = Buffers::new();
    assert_eq!(accept(&mut buffers, &first, 0), Ok(None));
    assert_eq!(
        accept(&mut buffers, &overlapping, 1),
        Err(Ipv6Error::Ip(IpError::OverlappingFragment))
    );
}

#[test]
fn a_piece_that_repeats_itself_exactly_is_a_duplicate_and_not_an_overlap() {
    let first = piece(0, true, 1, 16);
    let mut buffers = Buffers::new();
    assert_eq!(accept(&mut buffers, &first, 0), Ok(None));
    assert_eq!(accept(&mut buffers, &first, 1), Ok(None));
    let second = piece(16, false, 1, 16);
    assert!(
        accept(&mut buffers, &second, 2)
            .expect("it completes")
            .is_some()
    );
}

#[test]
fn two_datagrams_of_one_pair_are_kept_apart_by_the_identification() {
    let first = piece(0, true, 1, 16);
    let other = piece(0, true, 2, 16);
    let mut buffers = Buffers::new();
    assert_eq!(accept(&mut buffers, &first, 0), Ok(None));
    assert_eq!(accept(&mut buffers, &other, 1), Ok(None));
    assert_eq!(buffers.len(), 2);
}

#[test]
fn the_key_is_the_triple_rfc_8200_names_and_not_the_protocol() {
    let bytes = piece(0, true, 1, 16);
    let parsed = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = parsed.upper_layer().expect("a fragment header");
    let piece = piece_of(parsed, upper).expect("a piece");
    assert_eq!(
        piece,
        Piece {
            source: IpAddr::V6(HOST),
            destination: IpAddr::V6(PEER),
            // The upper layer is UDP; what goes in the key is the
            // constant, so that two fragments meet whatever their
            // fragment header says follows them.
            protocol: Protocol::FRAGMENT,
            identification: 1,
            offset: 0,
            more: true,
            payload: &DATAGRAM[..16],
        }
    );
}

#[test]
fn a_packet_with_no_fragment_header_is_no_piece() {
    let bytes = packet(HOST, PEER, Protocol::UDP, DEFAULT_HOP_LIMIT, &DATAGRAM);
    let parsed = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = parsed.upper_layer().expect("no chain");
    assert_eq!(piece_of(parsed, upper), None);
}

#[test]
fn releasing_forgets_a_half_assembled_datagram() {
    let first = piece(0, true, 1, 16);
    let mut buffers = Buffers::new();
    assert_eq!(accept(&mut buffers, &first, 0), Ok(None));
    assert_eq!(buffers.len(), 1);

    let parsed = Packet::parse(&first).expect("a well-formed packet");
    let upper = parsed.upper_layer().expect("a fragment header");
    release(&mut buffers, parsed, upper);
    assert!(buffers.is_empty());

    // Releasing a packet that is no piece is not an error and does
    // nothing.
    let whole = packet(HOST, PEER, Protocol::UDP, DEFAULT_HOP_LIMIT, &DATAGRAM);
    let parsed = Packet::parse(&whole).expect("a well-formed packet");
    let upper = parsed.upper_layer().expect("no chain");
    release(&mut buffers, parsed, upper);
    assert!(buffers.is_empty());
}

#[test]
fn a_datagram_longer_than_the_buffer_is_refused() {
    let mut body = fragment_header(Protocol::UDP, 1024, true, 1);
    body.extend_from_slice(&[1; 16]);
    let bytes = packet(HOST, PEER, Protocol::FRAGMENT, DEFAULT_HOP_LIMIT, &body);
    let mut buffers = Buffers::new();
    assert_eq!(
        accept(&mut buffers, &bytes, 0),
        Err(Ipv6Error::Ip(IpError::TooLarge(1040)))
    );
}

#[test]
fn a_half_assembled_datagram_is_dropped_at_its_deadline() {
    let first = piece(0, true, 1, 16);
    let mut buffers = Buffers::new();
    assert_eq!(accept(&mut buffers, &first, 0), Ok(None));
    let deadline = buffers.poll_at().expect("a deadline");
    assert_eq!(buffers.poll(deadline), 1);
    assert!(buffers.is_empty());
}
