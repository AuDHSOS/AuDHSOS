// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::ring`.

use crate::ring::{RING_CAPACITY, RING_HEADER_LEN, RING_LEN, Ring, SOCKET_PAGE_LEN, SocketPage};

#[test]
fn a_fresh_ring_says_it_holds_nothing() {
    let ring = Ring::new();
    let header = ring.header();
    assert_eq!(header.write_seq, 0);
    assert_eq!(header.read_seq, 0);
    assert_eq!(header.capacity, RING_CAPACITY);
    assert_eq!(ring.free(), RING_CAPACITY);
    assert!(ring.is_empty());
}

#[test]
fn the_page_is_two_rings_of_the_length_the_layout_fixes() {
    assert_eq!(
        RING_LEN,
        RING_HEADER_LEN + usize::try_from(RING_CAPACITY).unwrap_or(0)
    );
    assert_eq!(SOCKET_PAGE_LEN, 2 * RING_LEN);
}

#[test]
fn a_ring_that_was_used_reads_as_empty_after_it_is_initialized() {
    let page = SocketPage::new();
    assert_eq!(page.inbound.write(b"bytes"), 5);
    page.initialize();
    assert!(page.inbound.is_empty());
    assert_eq!(page.inbound.free(), RING_CAPACITY);
}

#[test]
fn a_ring_whose_capacity_is_not_the_fixed_one_takes_nothing() {
    let ring = Ring::new();
    ring.capacity
        .store(7, core::sync::atomic::Ordering::Release);
    assert_eq!(ring.write(b"bytes"), 0);
    let mut into = [0u8; 8];
    assert_eq!(ring.read(&mut into), 0);
}
