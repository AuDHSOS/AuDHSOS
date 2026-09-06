// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The outgoing queue: the order it keeps, the wrap it survives, and what
//! it does when it is full.

use test_support::generators::{range, vec};
use test_support::property::check;

use crate::queue::Queue;

/// The frames `queue` gives back, in order, into a buffer of `room`.
fn drain(queue: &mut Queue<'_>, room: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer = vec![0u8; room];
    while let Some(frame) = queue.pop_into(&mut buffer) {
        out.push(frame.to_vec());
    }
    out
}

#[test]
fn frames_come_out_in_the_order_they_went_in() {
    let mut memory = [0u8; 256];
    let mut queue = Queue::new(&mut memory);
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
    for frame in [&b"one"[..], b"two", b"three"] {
        assert!(queue.push(frame));
    }
    assert_eq!(queue.len(), 3);
    assert_eq!(queue.peek(), Some(&b"one"[..]));
    assert_eq!(
        drain(&mut queue, 64),
        vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
    );
    assert!(queue.is_empty());
    assert_eq!(queue.peek(), None);
}

#[test]
fn a_frame_that_no_longer_fits_behind_the_last_begins_at_the_front() {
    // Room for two records of twelve bytes and a few over.
    let mut memory = [0u8; 32];
    let mut queue = Queue::new(&mut memory);
    assert!(queue.push(&[1u8; 10]));
    assert!(queue.push(&[2u8; 10]));
    // Taking one out frees the front, and the next record wraps into it.
    let mut out = [0u8; 64];
    assert_eq!(queue.pop_into(&mut out), Some(&[1u8; 10][..]));
    assert!(queue.push(&[3u8; 6]));
    assert_eq!(queue.len(), 2);
    assert_eq!(queue.pop_into(&mut out), Some(&[2u8; 10][..]));
    assert_eq!(queue.pop_into(&mut out), Some(&[3u8; 6][..]));
    assert!(queue.is_empty());
}

#[test]
fn a_full_queue_drops_the_newest_and_counts_it() {
    let mut memory = [0u8; 16];
    let mut queue = Queue::new(&mut memory);
    assert!(queue.push(&[1u8; 6]));
    assert!(queue.push(&[2u8; 6]));
    assert!(!queue.push(&[3u8; 6]));
    assert_eq!(queue.dropped(), 1);
    assert_eq!(queue.len(), 2);
    let mut out = [0u8; 16];
    assert_eq!(queue.pop_into(&mut out), Some(&[1u8; 6][..]));
    assert_eq!(queue.pop_into(&mut out), Some(&[2u8; 6][..]));
}

#[test]
fn a_frame_longer_than_the_buffer_it_is_taken_into_stays_where_it_is() {
    let mut memory = [0u8; 64];
    let mut queue = Queue::new(&mut memory);
    assert!(queue.push(b"a long enough frame"));
    let mut small = [0u8; 4];
    assert_eq!(queue.pop_into(&mut small), None);
    assert_eq!(queue.len(), 1);
    let mut big = [0u8; 64];
    assert_eq!(queue.pop_into(&mut big), Some(&b"a long enough frame"[..]));
}

#[test]
fn a_frame_longer_than_a_record_may_be_is_refused() {
    let mut memory = vec![0u8; 200_000];
    let mut queue = Queue::new(&mut memory);
    let huge = vec![0u8; 65_535];
    assert!(!queue.push(&huge));
    assert_eq!(queue.dropped(), 1);
    let large = vec![0u8; 70_000];
    assert!(!queue.push(&large));
    assert_eq!(queue.dropped(), 2);
}

#[test]
fn an_empty_frame_is_a_frame() {
    let mut memory = [0u8; 16];
    let mut queue = Queue::new(&mut memory);
    assert!(queue.push(&[]));
    assert_eq!(queue.len(), 1);
    let mut out = [0u8; 4];
    assert_eq!(queue.pop_into(&mut out), Some(&[][..]));
}

#[test]
fn the_memory_comes_back() {
    let mut memory = [0u8; 16];
    let queue = Queue::new(&mut memory);
    assert_eq!(queue.into_bytes().len(), 16);
}

#[test]
fn whatever_goes_in_comes_out_in_the_same_order_however_it_wraps() {
    check(
        "stack_queue_keeps_its_order",
        &vec(range(1u8..=40), 1..=24),
        |lengths| {
            let mut memory = [0u8; 96];
            let mut queue = Queue::new(&mut memory);
            let mut expected: Vec<Vec<u8>> = Vec::new();
            let mut out = [0u8; 64];
            for (index, len) in lengths.iter().enumerate() {
                let byte = u8::try_from(index & 0xFF).unwrap_or(0);
                let frame = vec![byte; usize::from(*len)];
                if queue.push(&frame) {
                    expected.push(frame);
                }
                // Every other push, take one out, so that the ring wraps.
                if index % 2 == 1
                    && let Some(taken) = queue.pop_into(&mut out)
                {
                    let first = expected.remove(0);
                    assert_eq!(taken, first.as_slice(), "a frame came out of turn");
                }
            }
            while let Some(taken) = queue.pop_into(&mut out) {
                let first = expected.remove(0);
                assert_eq!(taken, first.as_slice(), "a frame came out of turn");
            }
            assert!(expected.is_empty(), "a frame was lost");
            Ok(())
        },
    );
}
