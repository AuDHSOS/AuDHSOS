// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The receive buffer: bytes in order, bytes out of order, and the gaps
//! between them.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test sizes a buffer by hand"
)]

use crate::recv::{MAX_HOLES, RecvBuffer};

/// Reads everything that is ready.
fn drain(buffer: &mut RecvBuffer<'_>) -> Vec<u8> {
    let mut out = vec![0u8; buffer.ready()];
    let taken = buffer.read(&mut out);
    out.truncate(taken);
    out
}

#[test]
fn bytes_that_arrive_in_order_are_ready_at_once() {
    let mut memory = [0u8; 16];
    let mut buffer = RecvBuffer::new(&mut memory);
    assert!(buffer.is_empty());
    assert_eq!(buffer.window(), 16);
    assert_eq!(buffer.accept(0, b"abc"), 3);
    assert_eq!(buffer.ready(), 3);
    assert_eq!(buffer.window(), 13);
    assert_eq!(buffer.contiguous(), b"abc");
    assert_eq!(drain(&mut buffer), b"abc");
    assert_eq!(buffer.window(), 16);
}

#[test]
fn a_segment_out_of_order_waits_for_the_gap_in_front_of_it_to_close() {
    let mut memory = [0u8; 16];
    let mut buffer = RecvBuffer::new(&mut memory);
    // The second segment arrives first.
    assert_eq!(buffer.accept(3, b"def"), 0);
    assert!(buffer.has_early_bytes());
    assert!(buffer.is_empty());
    // The window still counts it as room the peer may fill.
    assert_eq!(buffer.window(), 16);
    // The first closes the gap, and both become readable at once.
    assert_eq!(buffer.accept(0, b"abc"), 6);
    assert!(!buffer.has_early_bytes());
    assert_eq!(drain(&mut buffer), b"abcdef");
}

#[test]
fn two_ranges_grow_together_when_the_byte_between_them_arrives() {
    let mut memory = [0u8; 16];
    let mut buffer = RecvBuffer::new(&mut memory);
    buffer.accept(1, b"b");
    buffer.accept(3, b"d");
    assert!(buffer.has_early_bytes());
    // The byte in front takes the first range with it, and the byte that
    // closes the second gap takes the other.
    assert_eq!(buffer.accept(0, b"a"), 2);
    assert_eq!(buffer.accept(2, b"c"), 4);
    assert!(!buffer.has_early_bytes());
    assert_eq!(drain(&mut buffer), b"abcd");
}

#[test]
fn a_range_that_touches_the_run_is_absorbed_into_it() {
    let mut memory = [0u8; 16];
    let mut buffer = RecvBuffer::new(&mut memory);
    buffer.accept(2, b"cd");
    assert_eq!(buffer.accept(0, b"ab"), 4);
    assert_eq!(drain(&mut buffer), b"abcd");
}

#[test]
fn a_segment_that_repeats_what_is_already_there_changes_nothing() {
    let mut memory = [0u8; 16];
    let mut buffer = RecvBuffer::new(&mut memory);
    buffer.accept(0, b"abcd");
    assert_eq!(buffer.accept(0, b"abcd"), 4);
    assert_eq!(buffer.accept(1, b"bc"), 4);
    assert_eq!(drain(&mut buffer), b"abcd");
}

#[test]
fn a_segment_that_overlaps_the_end_of_the_run_adds_only_what_is_new() {
    let mut memory = [0u8; 16];
    let mut buffer = RecvBuffer::new(&mut memory);
    buffer.accept(0, b"abcd");
    assert_eq!(buffer.accept(2, b"cdef"), 6);
    assert_eq!(drain(&mut buffer), b"abcdef");
}

#[test]
fn bytes_beyond_the_window_are_not_taken() {
    let mut memory = [0u8; 4];
    let mut buffer = RecvBuffer::new(&mut memory);
    assert_eq!(buffer.accept(0, b"abcdef"), 4);
    assert_eq!(drain(&mut buffer), b"abcd");
    assert_eq!(buffer.accept(9, b"z"), 0);
    assert!(!buffer.has_early_bytes());
}

#[test]
fn a_range_that_does_not_fit_the_list_is_forgotten_and_sent_again() {
    let mut memory = [0u8; 64];
    let mut buffer = RecvBuffer::new(&mut memory);
    // One range per hole, each with a gap in front of it.
    for hole in 0..MAX_HOLES {
        let at = 2 + hole * 2;
        assert_eq!(buffer.accept(at, b"x"), 0);
    }
    let over = 2 + MAX_HOLES * 2;
    assert_eq!(buffer.accept(over, b"y"), 0);
    // Closing every gap yields the ranges that were remembered; the one
    // that was not stops the run one byte short of it, and the peer sends
    // it again.
    let filler = vec![b'.'; over];
    assert_eq!(buffer.accept(0, &filler), over);
}

#[test]
fn reading_makes_room_and_moves_the_ranges_with_it() {
    let mut memory = [0u8; 8];
    let mut buffer = RecvBuffer::new(&mut memory);
    buffer.accept(0, b"abcd");
    buffer.accept(6, b"g");
    assert_eq!(buffer.window(), 4);
    let mut out = [0u8; 2];
    assert_eq!(buffer.read(&mut out), 2);
    assert_eq!(&out, b"ab");
    assert_eq!(buffer.window(), 6);
    // The range moved down with the front, so the gap is now four bytes.
    assert_eq!(buffer.accept(2, b"efg"), 5);
    assert_eq!(drain(&mut buffer), b"cdefg");
}

#[test]
fn the_ring_wraps_without_cutting_a_read_in_two() {
    let mut memory = [0u8; 8];
    let mut buffer = RecvBuffer::new(&mut memory);
    buffer.accept(0, b"abcdef");
    let mut out = [0u8; 4];
    assert_eq!(buffer.read(&mut out), 4);
    assert_eq!(&out, b"abcd");
    // The front is now at position four with two bytes behind it, so four
    // more wrap around the end.
    assert_eq!(buffer.accept(2, b"ghij"), 6);
    // The first run stops where the memory does; the rest is the next.
    assert_eq!(buffer.contiguous(), b"efgh");
    assert_eq!(drain(&mut buffer), b"efghij");
}

#[test]
fn a_buffer_of_no_bytes_takes_nothing() {
    let mut memory = [0u8; 0];
    let mut buffer = RecvBuffer::new(&mut memory);
    assert_eq!(buffer.accept(0, b"a"), 0);
    assert_eq!(buffer.window(), 0);
    assert_eq!(buffer.contiguous(), b"");
    assert_eq!(buffer.into_bytes().len(), 0);
}

#[test]
fn reading_more_than_is_there_reads_what_is_there() {
    let mut memory = [0u8; 8];
    let mut buffer = RecvBuffer::new(&mut memory);
    buffer.accept(0, b"ab");
    let mut out = [0u8; 8];
    assert_eq!(buffer.read(&mut out), 2);
    assert_eq!(buffer.read(&mut out), 0);
}
