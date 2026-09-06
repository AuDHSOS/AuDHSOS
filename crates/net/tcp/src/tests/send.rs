// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The send buffer: what it takes, what it hands out, and what it drops.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test sizes a buffer by hand"
)]

use crate::send::SendBuffer;

#[test]
fn what_goes_in_comes_out_in_order() {
    let mut memory = [0u8; 16];
    let mut buffer = SendBuffer::new(&mut memory);
    assert!(buffer.is_empty());
    assert_eq!(buffer.capacity(), 16);
    assert_eq!(buffer.free(), 16);
    assert_eq!(buffer.write(b"hello"), 5);
    assert_eq!(buffer.len(), 5);
    assert_eq!(buffer.contiguous(0, 16), b"hello");
    assert_eq!(buffer.contiguous(2, 16), b"llo");
    assert_eq!(buffer.contiguous(5, 16), b"");
    assert_eq!(buffer.contiguous(9, 16), b"");
}

#[test]
fn a_write_takes_what_fits_and_says_how_much() {
    let mut memory = [0u8; 4];
    let mut buffer = SendBuffer::new(&mut memory);
    assert_eq!(buffer.write(b"abcdef"), 4);
    assert_eq!(buffer.contiguous(0, 8), b"abcd");
    assert_eq!(buffer.write(b"g"), 0);
}

#[test]
fn an_acknowledgment_makes_room_at_the_front() {
    let mut memory = [0u8; 8];
    let mut buffer = SendBuffer::new(&mut memory);
    buffer.write(b"abcdefgh");
    assert_eq!(buffer.acknowledge(3), 3);
    assert_eq!(buffer.len(), 5);
    assert_eq!(buffer.free(), 3);
    assert_eq!(buffer.contiguous(0, 8), b"defgh");
    // More than is there is as much as is there.
    assert_eq!(buffer.acknowledge(99), 5);
    assert!(buffer.is_empty());
    assert_eq!(buffer.contiguous(0, 8), b"");
}

#[test]
fn a_run_stops_at_the_end_of_the_memory_and_begins_again_at_its_front() {
    let mut memory = [0u8; 8];
    let mut buffer = SendBuffer::new(&mut memory);
    buffer.write(b"abcdef");
    buffer.acknowledge(5);
    // The front is now at position five with one byte behind it, so a
    // write of four wraps around the end.
    assert_eq!(buffer.write(b"wxyz"), 4);
    assert_eq!(buffer.len(), 5);
    // The first run stops where the memory does; the rest is the next one.
    assert_eq!(buffer.contiguous(0, 8), b"fwx");
    assert_eq!(buffer.contiguous(3, 8), b"yz");
    let mut seen = Vec::new();
    let mut offset = 0;
    while offset < buffer.len() {
        let run = buffer.contiguous(offset, 8);
        seen.extend_from_slice(run);
        offset += run.len();
    }
    assert_eq!(seen, b"fwxyz");
}

#[test]
fn a_run_is_never_longer_than_the_limit_it_was_asked_for() {
    let mut memory = [0u8; 16];
    let mut buffer = SendBuffer::new(&mut memory);
    buffer.write(b"abcdefgh");
    assert_eq!(buffer.contiguous(0, 3), b"abc");
    assert_eq!(buffer.contiguous(0, 0), b"");
}

#[test]
fn a_buffer_of_no_bytes_holds_nothing_and_says_so() {
    let mut memory = [0u8; 0];
    let mut buffer = SendBuffer::new(&mut memory);
    assert_eq!(buffer.write(b"a"), 0);
    assert_eq!(buffer.contiguous(0, 8), b"");
    assert_eq!(buffer.acknowledge(1), 0);
    assert_eq!(buffer.into_bytes().len(), 0);
}
