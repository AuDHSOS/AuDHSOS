// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the tests of this crate build their devices out of.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use virtio_queue::F_VERSION_1;
use virtio_queue::doubles::RamQueue;

use crate::doubles::{RamDevice, RamFrames};
use crate::error::NetError;
use crate::features::{F_MAC, F_MRG_RXBUF, F_STATUS};
use crate::init::Net;
use crate::net::{HEADER_LEN, MAC_LEN};
use crate::queues::{Queues, Rings, Side, Vectors};

/// Descriptors of each queue the tests drive.
pub(crate) const QUEUE_SIZE: u16 = 8;

/// How many descriptors and buffers the test driver keeps track of.
pub(crate) const SLOTS: usize = 8;

/// Bytes of one frame buffer: the header and a frame of 2036 bytes, which
/// is more than the largest this driver's callers write.
pub(crate) const STRIDE: u32 = 2048;

/// The multiplier of the notification structure.
pub(crate) const MULTIPLIER: u32 = 4;

/// The hardware address of the test device.
pub(crate) const ADDRESS: [u8; MAC_LEN] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

/// What a well-behaved test device offers: what this driver wants, and
/// two bits it does not, so that every test sees a refusal happen.
pub(crate) const OFFERED: u64 = F_VERSION_1 | F_MAC | F_MRG_RXBUF | F_STATUS;

/// Where the rings of a test lie. The addresses are arbitrary and only
/// have to reach the registers unchanged.
pub(crate) const QUEUES: Queues = Queues {
    receive: Rings {
        descriptor_table: 0x1_0000,
        available_ring: 0x1_1000,
        used_ring: 0x1_2000,
        size: QUEUE_SIZE,
    },
    transmit: Rings {
        descriptor_table: 0x2_0000,
        available_ring: 0x2_1000,
        used_ring: 0x2_2000,
        size: QUEUE_SIZE,
    },
};

/// The vectors of a test: one for the configuration and one for each
/// queue.
pub(crate) const VECTORS: Vectors = Vectors {
    config: 0,
    receive: 1,
    transmit: 2,
};

/// Where the receive buffers of a test lie.
pub(crate) const RECEIVE_BASE: u64 = 0x10_0000;

/// Where the transmit buffers lie.
pub(crate) const TRANSMIT_BASE: u64 = 0x20_0000;

/// A device that behaves, offering `OFFERED`.
pub(crate) fn device() -> RamDevice {
    RamDevice::new(OFFERED, QUEUE_SIZE, ADDRESS)
}

/// A driver brought up on `registers`.
pub(crate) fn live(registers: &mut RamDevice) -> Net<SLOTS> {
    let mut net = Net::new(MULTIPLIER);
    net.reset(registers);
    assert!(net.is_reset(registers));
    net.initialize(registers, &QUEUES, &VECTORS)
        .expect("the device comes up");
    net
}

/// A queue over memory of `QUEUE_SIZE` descriptors.
pub(crate) fn queue() -> (RamQueue, virtio_queue::Queue<1>) {
    let mut memory = RamQueue::new(QUEUE_SIZE);
    let queue = virtio_queue::Queue::new(&mut memory, QUEUE_SIZE).expect("a queue");
    (memory, queue)
}

/// The frame buffers of one side, as many as the queue has descriptors.
pub(crate) fn frames(side: Side) -> RamFrames {
    let base = match side {
        Side::Receive => RECEIVE_BASE,
        Side::Transmit => TRANSMIT_BASE,
    };
    RamFrames::new(QUEUE_SIZE, STRIDE, base)
}

/// The header a device writes before a received frame: every field zero,
/// which is what a device without an offload writes.
pub(crate) fn header() -> [u8; HEADER_LEN] {
    [0u8; HEADER_LEN]
}

/// The error a call refused with.
pub(crate) fn refusal<T: core::fmt::Debug>(result: Result<T, NetError>) -> NetError {
    match result {
        Err(error) => error,
        Ok(value) => panic!("the call answered {value:?} rather than refusing"),
    }
}
