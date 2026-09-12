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

use crate::blk::{Blk, Rings, Vectors};
use crate::doubles::RamDevice;
use crate::error::BlkError;
use crate::features::{F_FLUSH, F_SEG_MAX, F_TOPOLOGY};

/// Descriptors of the queue the tests drive.
pub(crate) const QUEUE_SIZE: u16 = 8;

/// Sectors the test device has.
pub(crate) const CAPACITY: u64 = 2048;

/// The multiplier of the notification structure.
pub(crate) const MULTIPLIER: u32 = 4;

/// What a well-behaved test device offers: what this driver wants, and
/// two bits it does not, so that every test sees a refusal happen.
pub(crate) const OFFERED: u64 = F_VERSION_1 | F_FLUSH | F_SEG_MAX | F_TOPOLOGY;

/// Where the rings of a test lie. The addresses are arbitrary and only
/// have to reach the registers unchanged.
pub(crate) const RINGS: Rings = Rings {
    descriptor_table: 0x1_0000,
    available_ring: 0x1_1000,
    used_ring: 0x1_2000,
    size: QUEUE_SIZE,
};

/// The vectors of a test: one for the queue and one for the
/// configuration.
pub(crate) const VECTORS: Vectors = Vectors {
    config: 0,
    queue: 1,
};

/// A device that behaves, offering `OFFERED`.
pub(crate) fn device() -> RamDevice {
    RamDevice::new(OFFERED, QUEUE_SIZE, CAPACITY)
}

/// A driver brought up on `registers`.
pub(crate) fn live(registers: &mut RamDevice) -> Blk {
    let mut blk = Blk::new(MULTIPLIER);
    blk.reset(registers);
    assert!(blk.is_reset(registers));
    blk.initialize(registers, &RINGS, &VECTORS)
        .expect("the device comes up");
    blk
}

/// A queue over memory of `QUEUE_SIZE` descriptors.
pub(crate) fn queue() -> (RamQueue, virtio_queue::Queue<1>) {
    let mut memory = RamQueue::new(QUEUE_SIZE);
    let queue = virtio_queue::Queue::new(&mut memory, QUEUE_SIZE).expect("a queue");
    (memory, queue)
}

/// The error a call refused with.
pub(crate) fn refusal<T: core::fmt::Debug>(result: Result<T, BlkError>) -> BlkError {
    match result {
        Err(error) => error,
        Ok(value) => panic!("the call answered {value:?} rather than refusing"),
    }
}
