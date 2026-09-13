// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The byte ring a socket's data travels through.

#![allow(clippy::indexing_slicing)]

use net_wire::{IpAddr, Ipv4Addr};
use user_proto::ring::{RING_CAPACITY, Ring, SocketPage};
use user_proto::socket::{DATAGRAM_HEADER_LEN, Endpoint, datagram_header, read_datagram_header};

#[test]
fn what_is_written_is_what_is_read() {
    let ring = Ring::new();
    assert_eq!(ring.write(b"a stream of bytes"), 17);
    let mut into = [0u8; 32];
    assert_eq!(ring.read(&mut into), 17);
    assert_eq!(&into[..17], b"a stream of bytes");
    assert!(ring.is_empty());
}

#[test]
fn a_ring_exactly_full_takes_no_further_byte_and_counts_what_it_lost() {
    let ring = Ring::new();
    let bytes = vec![7u8; usize::try_from(RING_CAPACITY).unwrap_or(0)];
    assert_eq!(ring.write(&bytes), bytes.len());
    assert_eq!(ring.free(), 0);
    assert_eq!(ring.write(b"one more"), 0);
    assert_eq!(ring.take_dropped(), 8);
    assert_eq!(ring.take_dropped(), 0, "a gap is reported once");
}

#[test]
fn a_reader_that_stops_stops_the_writer() {
    let ring = Ring::new();
    let bytes = vec![3u8; usize::try_from(RING_CAPACITY).unwrap_or(0) + 16];
    let taken = ring.write(&bytes);
    assert_eq!(taken, usize::try_from(RING_CAPACITY).unwrap_or(0));
    let mut into = [0u8; 16];
    assert_eq!(ring.read(&mut into), 16);
    assert_eq!(ring.write(&[1u8; 16]), 16, "reading made room");
}

#[test]
fn the_bytes_come_back_in_order_across_the_wrap() {
    let ring = Ring::new();
    let capacity = usize::try_from(RING_CAPACITY).unwrap_or(0);
    let filler = vec![0u8; capacity - 4];
    assert_eq!(ring.write(&filler), filler.len());
    let mut drop = vec![0u8; capacity - 4];
    assert_eq!(ring.read(&mut drop), filler.len());
    assert_eq!(ring.write(b"across the wrap"), 15);
    let mut into = [0u8; 15];
    assert_eq!(ring.read(&mut into), 15);
    assert_eq!(&into, b"across the wrap");
}

#[test]
fn a_read_of_an_empty_ring_takes_nothing() {
    let ring = Ring::new();
    let mut into = [0u8; 8];
    assert_eq!(ring.read(&mut into), 0);
}

#[test]
fn a_page_of_two_rings_keeps_them_apart() {
    let page = SocketPage::new();
    page.initialize();
    assert_eq!(page.inbound.write(b"in"), 2);
    assert_eq!(page.outbound.write(b"out"), 3);
    let mut into = [0u8; 8];
    assert_eq!(page.inbound.read(&mut into), 2);
    assert_eq!(&into[..2], b"in");
    assert_eq!(page.outbound.read(&mut into), 3);
    assert_eq!(&into[..3], b"out");
}

#[test]
fn a_datagram_record_says_where_it_came_from_and_how_long_it_is() {
    let from = Endpoint::new(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 3)), 53);
    let header = datagram_header(from, 42);
    assert_eq!(header.len(), DATAGRAM_HEADER_LEN);
    assert_eq!(read_datagram_header(&header), Some((from, 42)));
}

#[test]
fn a_record_of_a_family_that_is_neither_is_refused() {
    let mut header = [0u8; DATAGRAM_HEADER_LEN];
    header[4] = 9;
    assert_eq!(read_datagram_header(&header), None);
}
