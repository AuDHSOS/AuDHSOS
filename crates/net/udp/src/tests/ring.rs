// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The receive ring: its records, its wrap, and what it does when it is
//! full.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test sizes a buffer from the record length it computed"
)]

use net_wire::{IpAddr, IpVersion, Ipv4Addr, Ipv6Addr, Port};

use crate::ring::{DropReason, Received, Ring, read_record, record_len, write_record};

/// The other host, over IPv4.
const PEER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

/// This host, over IPv4.
const HERE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));

/// The other host, over IPv6.
const PEER_V6: IpAddr = IpAddr::V6(Ipv6Addr::from_octets([
    0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x02,
]));

/// This host, over IPv6.
const HERE_V6: IpAddr = IpAddr::V6(Ipv6Addr::from_octets([
    0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01,
]));

/// The port every record in these tests came from.
const FROM: Port = Port::new(53);

#[test]
fn a_record_costs_its_payload_and_the_addresses_it_arrived_between() {
    assert_eq!(record_len(IpVersion::V4, 0), 13);
    assert_eq!(record_len(IpVersion::V4, 4), 17);
    assert_eq!(record_len(IpVersion::V6, 0), 37);
    assert_eq!(record_len(IpVersion::V6, 4), 41);
}

#[test]
fn a_datagram_comes_back_with_the_addresses_it_arrived_between() {
    let mut memory = [0u8; 64];
    let mut ring = Ring::new(&mut memory);
    assert!(ring.is_empty());
    assert_eq!(ring.capacity(), 64);
    ring.push(PEER, HERE, FROM, b"answer").expect("room");
    assert_eq!(ring.len(), 1);
    assert_eq!(
        ring.peek(),
        Some(Received {
            source: PEER,
            destination: HERE,
            port: FROM,
            payload: b"answer",
        })
    );
    assert!(ring.discard());
    assert!(ring.is_empty());
    assert!(!ring.discard());
    assert_eq!(ring.peek(), None);
}

#[test]
fn an_ipv6_datagram_comes_back_with_both_of_its_addresses() {
    let mut memory = [0u8; 64];
    let mut ring = Ring::new(&mut memory);
    ring.push(PEER_V6, HERE_V6, FROM, b"six").expect("room");
    assert_eq!(
        ring.peek(),
        Some(Received {
            source: PEER_V6,
            destination: HERE_V6,
            port: FROM,
            payload: b"six",
        })
    );
}

#[test]
fn datagrams_come_back_in_the_order_they_arrived() {
    let mut memory = [0u8; 128];
    let mut ring = Ring::new(&mut memory);
    for payload in [b"one".as_slice(), b"two", b"three"] {
        ring.push(PEER, HERE, FROM, payload).expect("room");
    }
    assert_eq!(ring.len(), 3);
    for payload in [b"one".as_slice(), b"two", b"three"] {
        assert_eq!(ring.peek().expect("a datagram").payload, payload);
        assert!(ring.discard());
    }
    assert!(ring.is_empty());
}

#[test]
fn a_record_that_no_longer_fits_behind_the_last_begins_at_the_front() {
    // Three records of four payload bytes fit; the buffer has no room for
    // a fourth behind them.
    let one = record_len(IpVersion::V4, 4);
    let mut memory = vec![0u8; one * 3 + one / 2];
    let mut ring = Ring::new(&mut memory);
    for payload in [b"aaaa".as_slice(), b"bbbb", b"cccc"] {
        ring.push(PEER, HERE, FROM, payload).expect("room");
    }
    assert!(ring.discard());
    // The room at the end is too small, and the room at the front is not.
    ring.push(PEER, HERE, FROM, b"dddd").expect("the front");
    assert_eq!(ring.len(), 3);
    for payload in [b"bbbb".as_slice(), b"cccc", b"dddd"] {
        let received = ring.peek().expect("a datagram");
        assert_eq!(received.payload, payload);
        assert_eq!(received.payload.len(), 4);
        assert!(ring.discard());
    }
    assert!(ring.is_empty());
    // Emptied, it takes a record that fills it from the front again.
    ring.push(PEER, HERE, FROM, b"eeee").expect("room");
    assert_eq!(ring.peek().expect("a datagram").payload, b"eeee");
}

#[test]
fn a_full_ring_drops_the_newest_datagram_and_counts_it() {
    let one = record_len(IpVersion::V4, 4);
    let mut memory = vec![0u8; one * 2];
    let mut ring = Ring::new(&mut memory);
    ring.push(PEER, HERE, FROM, b"aaaa").expect("room");
    ring.push(PEER, HERE, FROM, b"bbbb").expect("room");
    assert_eq!(ring.push(PEER, HERE, FROM, b"cccc"), Err(DropReason::Full));
    assert_eq!(ring.dropped(), 1);
    assert_eq!(ring.len(), 2);
    // The two that were there are the two that stayed.
    assert_eq!(ring.peek().expect("a datagram").payload, b"aaaa");
    assert!(ring.discard());
    assert_eq!(ring.peek().expect("a datagram").payload, b"bbbb");
}

#[test]
fn a_wrapped_ring_that_is_full_drops_as_well() {
    let one = record_len(IpVersion::V4, 4);
    let mut memory = vec![0u8; one * 3];
    let mut ring = Ring::new(&mut memory);
    for payload in [b"aaaa".as_slice(), b"bbbb", b"cccc"] {
        ring.push(PEER, HERE, FROM, payload).expect("room");
    }
    assert!(ring.discard());
    ring.push(PEER, HERE, FROM, b"dddd").expect("the front");
    assert_eq!(ring.push(PEER, HERE, FROM, b"eeee"), Err(DropReason::Full));
    assert_eq!(ring.dropped(), 1);
}

#[test]
fn a_datagram_larger_than_the_ring_is_refused_at_entry() {
    let mut memory = [0u8; 20];
    let mut ring = Ring::new(&mut memory);
    assert_eq!(
        ring.push(PEER, HERE, FROM, &[0u8; 16]),
        Err(DropReason::TooLarge)
    );
    assert_eq!(ring.dropped(), 1);
    assert!(ring.is_empty());
    // An empty ring is no help: the record does not fit it either.
    assert_eq!(
        ring.push(PEER, HERE, FROM, &[0u8; 16]),
        Err(DropReason::TooLarge)
    );
    assert_eq!(ring.dropped(), 2);
}

#[test]
fn a_payload_longer_than_a_record_can_name_is_refused() {
    let payload = vec![0u8; usize::from(u16::MAX) + 1];
    let mut memory = vec![0u8; payload.len() + 64];
    let mut ring = Ring::new(&mut memory);
    assert_eq!(
        ring.push(PEER, HERE, FROM, &payload),
        Err(DropReason::TooLarge)
    );
}

#[test]
fn two_addresses_of_different_families_are_no_record() {
    let mut memory = [0u8; 64];
    let mut ring = Ring::new(&mut memory);
    assert_eq!(
        ring.push(PEER, HERE_V6, FROM, b"four"),
        Err(DropReason::MixedFamilies)
    );
    assert_eq!(ring.dropped(), 1);
    assert!(ring.is_empty());
}

#[test]
fn the_memory_comes_back_when_the_ring_is_given_up() {
    let mut memory = [0u8; 64];
    let mut ring = Ring::new(&mut memory);
    ring.push(PEER, HERE, FROM, b"four").expect("room");
    let bytes = ring.into_bytes();
    assert_eq!(bytes.len(), 64);
}

#[test]
fn a_record_needs_every_byte_of_its_slot() {
    let full = record_len(IpVersion::V4, 4);
    for len in 0..full {
        let mut slot = vec![0u8; len];
        assert_eq!(
            write_record(&mut slot, PEER, HERE, FROM, b"four"),
            None,
            "a slot of {len} bytes took a record of {full}"
        );
    }
    let mut slot = vec![0u8; full];
    assert_eq!(write_record(&mut slot, PEER, HERE, FROM, b"four"), Some(()));
}

#[test]
fn a_record_of_ipv6_addresses_needs_every_byte_of_its_slot() {
    let full = record_len(IpVersion::V6, 3);
    for len in 0..full {
        let mut slot = vec![0u8; len];
        assert_eq!(
            write_record(&mut slot, PEER_V6, HERE_V6, FROM, b"six"),
            None
        );
    }
    let mut slot = vec![0u8; full];
    assert_eq!(
        write_record(&mut slot, PEER_V6, HERE_V6, FROM, b"six"),
        Some(())
    );
}

#[test]
fn a_payload_longer_than_a_record_can_name_is_no_record_either() {
    let payload = vec![0u8; usize::from(u16::MAX) + 1];
    let mut slot = vec![0u8; payload.len() + 64];
    assert_eq!(write_record(&mut slot, PEER, HERE, FROM, &payload), None);
}

#[test]
fn a_record_that_was_cut_short_reads_back_as_none() {
    let full = record_len(IpVersion::V4, 4);
    let mut slot = vec![0u8; full];
    write_record(&mut slot, PEER, HERE, FROM, b"four").expect("a record");
    assert_eq!(read_record(&slot).map(|(_, len)| len), Some(full));
    for len in 0..full {
        assert_eq!(
            read_record(&slot[..len]).map(|(_, taken)| taken),
            None,
            "{len} bytes read as a whole record"
        );
    }
}

#[test]
fn an_ipv6_record_that_was_cut_short_reads_back_as_none() {
    let full = record_len(IpVersion::V6, 3);
    let mut slot = vec![0u8; full];
    write_record(&mut slot, PEER_V6, HERE_V6, FROM, b"six").expect("a record");
    assert_eq!(read_record(&slot).map(|(_, len)| len), Some(full));
    for len in 0..full {
        assert_eq!(read_record(&slot[..len]).map(|(_, taken)| taken), None);
    }
}

#[test]
fn a_tag_of_neither_family_is_no_record() {
    let full = record_len(IpVersion::V4, 4);
    let mut slot = vec![0u8; full];
    write_record(&mut slot, PEER, HERE, FROM, b"four").expect("a record");
    slot[0] = 5;
    assert_eq!(read_record(&slot).map(|(_, len)| len), None);
}
