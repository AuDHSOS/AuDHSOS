// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Receive: the buffers that are in the ring from the start, the frame
//! behind the header, and the buffer that goes back in the same call.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use crate::error::NetError;
use crate::net::HEADER_LEN;
use crate::queues::Side;

use super::support::{QUEUE_SIZE, STRIDE, device, frames, header, live, queue, refusal};

/// The frame a test sends through the device.
const FRAME: &[u8] = b"a frame of the link";

#[test]
fn every_buffer_is_in_the_available_ring_after_the_fill() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let area = frames(Side::Receive);
    let filled = net
        .fill(&mut queue, &mut memory, &area)
        .expect("the buffers go in");
    assert_eq!(filled, QUEUE_SIZE);
    assert_eq!(memory.available_index(), QUEUE_SIZE);
    assert_eq!(queue.free_count(), 0);
    for slot in 0..QUEUE_SIZE {
        let head = memory.available_entry(slot);
        let descriptor = memory.descriptor(head).expect("a descriptor");
        assert_eq!(descriptor.length, STRIDE);
        assert_eq!(descriptor.flags, virtio_queue::DESC_F_WRITE);
    }
}

#[test]
fn a_fill_that_finds_every_buffer_posted_adds_nothing() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let area = frames(Side::Receive);
    let _first = net.fill(&mut queue, &mut memory, &area);
    assert_eq!(net.fill(&mut queue, &mut memory, &area), Ok(0));
}

#[test]
fn a_used_element_yields_the_frame_behind_the_header() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Receive);
    net.fill(&mut queue, &mut memory, &area).expect("filled");
    let head = memory.available_entry(0);
    let written = area.deliver(0, &header(), FRAME);
    memory.complete(u32::from(head), written);
    let mut into = [0u8; 2048];
    let frame = net
        .receive(&mut queue, &mut memory, &area, &mut into)
        .expect("a frame")
        .expect("one was there");
    assert_eq!(frame, FRAME);
}

#[test]
fn the_buffer_goes_back_into_the_ring_in_the_call_that_took_it() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Receive);
    net.fill(&mut queue, &mut memory, &area).expect("filled");
    let head = memory.available_entry(0);
    let written = area.deliver(0, &header(), FRAME);
    memory.complete(u32::from(head), written);
    let mut into = [0u8; 2048];
    let _frame = net.receive(&mut queue, &mut memory, &area, &mut into);
    assert_eq!(queue.free_count(), 0, "a descriptor was left free");
    assert_eq!(memory.available_index(), QUEUE_SIZE + 1);
    let again = memory.available_entry(QUEUE_SIZE % QUEUE_SIZE);
    let descriptor = memory.descriptor(again).expect("a descriptor");
    assert_eq!(descriptor.length, STRIDE);
}

#[test]
fn a_receive_with_nothing_in_the_used_ring_answers_nothing() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let area = frames(Side::Receive);
    net.fill(&mut queue, &mut memory, &area).expect("filled");
    let mut into = [0u8; 2048];
    assert_eq!(
        net.receive(&mut queue, &mut memory, &area, &mut into),
        Ok(None)
    );
}

#[test]
fn a_used_element_below_the_header_is_refused_and_the_buffer_comes_back() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Receive);
    net.fill(&mut queue, &mut memory, &area).expect("filled");
    let head = memory.available_entry(0);
    let _written = area.deliver(0, &header(), FRAME);
    memory.complete(u32::from(head), 4);
    let mut into = [0u8; 2048];
    let error = refusal(net.receive(&mut queue, &mut memory, &area, &mut into));
    assert_eq!(error, NetError::ShortFrame(4));
    assert_eq!(queue.free_count(), 0, "the buffer did not go back");
}

#[test]
fn a_frame_longer_than_the_callers_buffer_is_refused_and_the_buffer_comes_back() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Receive);
    net.fill(&mut queue, &mut memory, &area).expect("filled");
    let head = memory.available_entry(0);
    let long = std::vec![7u8; 64];
    let written = area.deliver(0, &header(), &long);
    memory.complete(u32::from(head), written);
    let mut into = [0u8; 16];
    let error = refusal(net.receive(&mut queue, &mut memory, &area, &mut into));
    assert_eq!(
        error,
        NetError::FrameTooLong {
            len: 64,
            capacity: 16
        }
    );
    assert_eq!(queue.free_count(), 0);
}

#[test]
fn a_used_element_naming_a_descriptor_the_driver_did_not_hand_out_is_refused() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let area = frames(Side::Receive);
    net.fill(&mut queue, &mut memory, &area).expect("filled");
    // Every descriptor is in a chain, so the one the driver has no buffer
    // for is one the queue itself refuses first.
    memory.complete(u32::from(QUEUE_SIZE), 32);
    let mut into = [0u8; 2048];
    let error = refusal(net.receive(&mut queue, &mut memory, &area, &mut into));
    assert_eq!(
        error,
        NetError::Queue(virtio_queue::QueueError::UnknownDescriptor(u32::from(
            QUEUE_SIZE
        )))
    );
}

#[test]
fn a_used_element_longer_than_the_buffer_is_refused_by_the_queue() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let area = frames(Side::Receive);
    net.fill(&mut queue, &mut memory, &area).expect("filled");
    let head = memory.available_entry(0);
    memory.complete(u32::from(head), STRIDE + 1);
    let mut into = [0u8; 4096];
    let error = refusal(net.receive(&mut queue, &mut memory, &area, &mut into));
    assert_eq!(
        error,
        NetError::Queue(virtio_queue::QueueError::UsedLength {
            reported: STRIDE + 1,
            writable: STRIDE
        })
    );
}

#[test]
fn an_area_of_more_buffers_than_the_driver_holds_is_refused() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let area = crate::doubles::RamFrames::new(QUEUE_SIZE * 2, STRIDE, 0x30_0000);
    let error = refusal(net.fill(&mut queue, &mut memory, &area));
    assert_eq!(error, NetError::Slots(QUEUE_SIZE * 2));
}

#[test]
fn a_frame_of_no_bytes_at_all_is_a_frame_of_no_bytes() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Receive);
    net.fill(&mut queue, &mut memory, &area).expect("filled");
    let head = memory.available_entry(0);
    let written = area.deliver(0, &header(), &[]);
    assert_eq!(written, HEADER_LEN as u32);
    memory.complete(u32::from(head), written);
    let mut into = [0u8; 64];
    let frame = net
        .receive(&mut queue, &mut memory, &area, &mut into)
        .expect("a frame")
        .expect("one was there");
    assert!(frame.is_empty());
}

#[test]
fn a_used_element_naming_a_buffer_the_area_does_not_hold_is_refused() {
    let mut registers = device();
    let mut net = live(&mut registers);
    let (mut memory, mut queue) = queue();
    let mut area = frames(Side::Receive);
    net.fill(&mut queue, &mut memory, &area).expect("filled");
    let head = memory.available_entry(QUEUE_SIZE - 1);
    let written = area.deliver(QUEUE_SIZE - 1, &header(), FRAME);
    memory.complete(u32::from(head), written);
    // A shorter area than the one the buffers were posted from: the
    // element names a buffer that area does not hold.
    let shorter = crate::doubles::RamFrames::new(1, STRIDE, 0x10_0000);
    let mut into = [0u8; 64];
    let error = refusal(net.receive(&mut queue, &mut memory, &shorter, &mut into));
    assert_eq!(error, NetError::UnknownBuffer(head));
}
