// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Transmit: the header, the one chain, the completions that are drained
//! first, and the refusal a caller retries.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use crate::error::NetError;
use crate::net::HEADER_LEN;
use crate::queues::Side;

use super::support::{QUEUE_SIZE, STRIDE, device, frames, live, queue, refusal};

/// The frame a test sends.
const FRAME: &[u8] = b"a frame of the link";

#[test]
fn a_frame_goes_out_behind_a_zeroed_header_as_one_chain() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Transmit);
    let notify = net
        .send(&mut queue, &mut memory, &mut area, FRAME)
        .expect("the frame goes out");
    assert!(notify, "a device that has not suppressed wants telling");
    let head = memory.available_entry(0);
    let descriptor = memory.descriptor(head).expect("a descriptor");
    assert_eq!(descriptor.flags, 0, "the device reads this buffer");
    assert_eq!(descriptor.length, (HEADER_LEN + FRAME.len()) as u32);
    let bytes = area.buffer(0);
    assert_eq!(&bytes[..HEADER_LEN], &[0u8; HEADER_LEN]);
    assert_eq!(&bytes[HEADER_LEN..HEADER_LEN + FRAME.len()], FRAME);
}

#[test]
fn a_frame_longer_than_a_buffer_is_refused_before_anything_is_written() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Transmit);
    let long = std::vec![9u8; STRIDE as usize];
    let error = refusal(net.send(&mut queue, &mut memory, &mut area, &long));
    assert_eq!(
        error,
        NetError::FrameTooLong {
            len: STRIDE as usize,
            capacity: STRIDE as usize - HEADER_LEN
        }
    );
    assert_eq!(memory.available_index(), 0);
    assert_eq!(area.buffer(0)[HEADER_LEN], 0);
}

#[test]
fn a_send_with_every_buffer_at_the_device_is_refused_and_changes_nothing() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Transmit);
    for _ in 0..QUEUE_SIZE {
        net.send(&mut queue, &mut memory, &mut area, FRAME)
            .expect("a buffer was free");
    }
    let error = refusal(net.send(&mut queue, &mut memory, &mut area, FRAME));
    assert_eq!(error, NetError::NoBuffer);
    assert_eq!(memory.available_index(), QUEUE_SIZE);
}

#[test]
fn the_completions_are_drained_before_the_next_send() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Transmit);
    for _ in 0..QUEUE_SIZE {
        net.send(&mut queue, &mut memory, &mut area, FRAME)
            .expect("a buffer was free");
    }
    let head = memory.available_entry(0);
    memory.complete(u32::from(head), 0);
    net.send(&mut queue, &mut memory, &mut area, FRAME)
        .expect("the drained buffer is free again");
    assert_eq!(memory.available_index(), QUEUE_SIZE + 1);
}

#[test]
fn a_drain_of_nothing_answers_nothing() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (memory, mut queue) = queue();
    assert_eq!(net.drain(&mut queue, &memory), Ok(0));
}

#[test]
fn a_drain_counts_what_came_back() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Transmit);
    for _ in 0..3 {
        net.send(&mut queue, &mut memory, &mut area, FRAME)
            .expect("a buffer was free");
    }
    for slot in 0..3 {
        let head = memory.available_entry(slot);
        memory.complete(u32::from(head), 0);
    }
    assert_eq!(net.drain(&mut queue, &memory), Ok(3));
}

#[test]
fn a_device_that_suppressed_notifications_is_not_told() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Transmit);
    memory.set_no_notify(true);
    let notify = net
        .send(&mut queue, &mut memory, &mut area, FRAME)
        .expect("the frame goes out");
    assert!(!notify);
}

#[test]
fn a_send_over_an_area_of_more_buffers_than_the_driver_holds_is_refused() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = crate::doubles::RamFrames::new(QUEUE_SIZE * 2, STRIDE, 0x40_0000);
    let error = refusal(net.send(&mut queue, &mut memory, &mut area, FRAME));
    assert_eq!(error, NetError::Slots(QUEUE_SIZE * 2));
}

#[test]
fn rings_of_a_size_the_caller_left_out_are_refused() {
    use crate::queues::{Queues, Rings};
    let mut registers = device();
    let mut net = crate::init::Net::<{ super::support::SLOTS }>::new(super::support::MULTIPLIER);
    net.reset(&mut registers);
    let queues = Queues {
        receive: Rings {
            size: 0,
            ..super::support::QUEUES.receive
        },
        transmit: super::support::QUEUES.transmit,
    };
    let error = refusal(net.initialize(&mut registers, &queues, &super::support::VECTORS));
    assert_eq!(error, NetError::QueueSize(0));
}
