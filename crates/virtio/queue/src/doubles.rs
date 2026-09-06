// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A virtqueue in plain memory that can also act as the device, and a
//! register file that can be scripted to refuse.

use std::cell::Cell;
use std::vec;
use std::vec::Vec;

use crate::descriptor::Descriptor;
use crate::device::{DeviceRegisters, STATUS_DEVICE_NEEDS_RESET, STATUS_FEATURES_OK};
use crate::memory::{
    QueueMemory, available_ring_bytes, descriptor_table_bytes, used_ring_bytes, write_u16,
    write_u32,
};

/// Offset of the flags in the available and in the used ring.
const FLAGS_AT: usize = 0;

/// Offset of the index in the available and in the used ring.
const INDEX_AT: usize = 2;

/// The three regions of a virtqueue as three byte vectors, with the
/// device's half of the exchange available to a test.
#[derive(Clone, Debug)]
pub struct RamQueue {
    /// Number of descriptors the regions were sized for.
    size: u16,
    /// The descriptor table.
    descriptors: Vec<u8>,
    /// The available ring.
    available: Vec<u8>,
    /// The used ring.
    used: Vec<u8>,
    /// How often [`QueueMemory::barrier`] was called.
    barriers: Cell<usize>,
}

impl RamQueue {
    /// Regions sized for a queue of `size` descriptors, all zero.
    #[must_use]
    pub fn new(size: u16) -> RamQueue {
        RamQueue {
            size,
            descriptors: vec![0; descriptor_table_bytes(size)],
            available: vec![0; available_ring_bytes(size)],
            used: vec![0; used_ring_bytes(size)],
            barriers: Cell::new(0),
        }
    }

    /// Regions of exactly the given lengths, for the case where one is
    /// too short for the queue that runs over it.
    #[must_use]
    pub fn with_lengths(size: u16, descriptors: usize, available: usize, used: usize) -> RamQueue {
        RamQueue {
            size,
            descriptors: vec![0; descriptors],
            available: vec![0; available],
            used: vec![0; used],
            barriers: Cell::new(0),
        }
    }

    /// How often the queue asked for a barrier.
    #[must_use]
    pub const fn barriers(&self) -> usize {
        self.barriers.get()
    }

    /// The index the driver has published.
    #[must_use]
    pub fn available_index(&self) -> u16 {
        peek_u16(&self.available, INDEX_AT)
    }

    /// The flags the driver has written into the available ring.
    #[must_use]
    pub fn available_flags(&self) -> u16 {
        peek_u16(&self.available, FLAGS_AT)
    }

    /// The head the driver put into available ring slot `slot`.
    #[must_use]
    pub fn available_entry(&self, slot: u16) -> u16 {
        let at = 4usize.saturating_add(usize::from(slot).saturating_mul(2));
        peek_u16(&self.available, at)
    }

    /// Descriptor `index` as the driver wrote it, or `None` when the
    /// table is shorter than that.
    #[must_use]
    pub fn descriptor(&self, index: u16) -> Option<Descriptor> {
        Descriptor::read(&self.descriptors, index).ok()
    }

    /// Shortens the descriptor table, which is how a memory that hands
    /// over less than it did before is made.
    pub fn truncate_descriptors(&mut self, length: usize) {
        self.descriptors.truncate(length);
    }

    /// Shortens the available ring.
    pub fn truncate_available(&mut self, length: usize) {
        self.available.truncate(length);
    }

    /// Acts as the device: gives the chain at `head` back with `length`
    /// bytes written, and advances the used index.
    pub fn complete(&mut self, head: u32, length: u32) {
        let slot = self.used_index() & self.size.wrapping_sub(1);
        let at = 4usize.saturating_add(usize::from(slot).saturating_mul(8));
        self.put_u32(at, head);
        self.put_u32(at.saturating_add(4), length);
        let next = self.used_index().wrapping_add(1);
        self.set_used_index(next);
    }

    /// The used index the device has reached.
    #[must_use]
    pub fn used_index(&self) -> u16 {
        peek_u16(&self.used, INDEX_AT)
    }

    /// Acts as the device: moves the used index without writing an
    /// element, which is how a used index that went backwards is made.
    pub fn set_used_index(&mut self, index: u16) {
        let _ = write_u16(
            &mut self.used,
            crate::error::Area::UsedRing,
            INDEX_AT,
            index,
        );
    }

    /// Acts as the device: sets or clears the no-notify flag.
    pub fn set_no_notify(&mut self, suppress: bool) {
        let flags = u16::from(suppress);
        let _ = write_u16(
            &mut self.used,
            crate::error::Area::UsedRing,
            FLAGS_AT,
            flags,
        );
    }

    /// Overwrites the flags and the next index of a descriptor, which is
    /// how a chain that does not end is made.
    pub fn scribble_link(&mut self, index: u16, flags: u16, next: u16) {
        let at = Descriptor::offset(index);
        let _ = write_u16(
            &mut self.descriptors,
            crate::error::Area::DescriptorTable,
            at.saturating_add(12),
            flags,
        );
        let _ = write_u16(
            &mut self.descriptors,
            crate::error::Area::DescriptorTable,
            at.saturating_add(14),
            next,
        );
    }

    /// Writes a little-endian `u32` into the used ring.
    fn put_u32(&mut self, at: usize, value: u32) {
        let _ = write_u32(&mut self.used, crate::error::Area::UsedRing, at, value);
    }
}

impl QueueMemory for RamQueue {
    fn descriptor_table(&self) -> &[u8] {
        &self.descriptors
    }

    fn descriptor_table_mut(&mut self) -> &mut [u8] {
        &mut self.descriptors
    }

    fn available_ring_mut(&mut self) -> &mut [u8] {
        &mut self.available
    }

    fn used_ring(&self) -> &[u8] {
        &self.used
    }

    fn barrier(&self) {
        self.barriers.set(self.barriers.get().saturating_add(1));
    }
}

/// A status and feature register file that records what was written and
/// can be told to refuse.
#[derive(Clone, Debug)]
pub struct ScriptedRegisters {
    /// The status register.
    status: u8,
    /// What the device offers.
    device_features: u64,
    /// What the driver last wrote.
    driver_features: u64,
    /// Whether a write of `FEATURES_OK` is dropped.
    rejects_features: bool,
    /// Every value written to the status register, in order.
    writes: Vec<u8>,
}

impl ScriptedRegisters {
    /// A device offering `device_features` and accepting every step.
    #[must_use]
    pub const fn new(device_features: u64) -> ScriptedRegisters {
        ScriptedRegisters {
            status: 0,
            device_features,
            driver_features: 0,
            rejects_features: false,
            writes: Vec::new(),
        }
    }

    /// Makes the device drop `FEATURES_OK` when the driver sets it.
    pub const fn reject_features(&mut self) {
        self.rejects_features = true;
    }

    /// Acts as the device: asks for a reset.
    pub const fn request_reset(&mut self) {
        self.status |= STATUS_DEVICE_NEEDS_RESET;
    }

    /// Every value written to the status register, in order.
    #[must_use]
    pub fn status_writes(&self) -> &[u8] {
        &self.writes
    }

    /// What the driver accepted.
    #[must_use]
    pub const fn driver_features(&self) -> u64 {
        self.driver_features
    }
}

impl DeviceRegisters for ScriptedRegisters {
    fn status(&self) -> u8 {
        self.status
    }

    fn set_status(&mut self, status: u8) {
        self.writes.push(status);
        self.status = if self.rejects_features {
            status & !STATUS_FEATURES_OK
        } else {
            status
        };
    }

    fn device_features(&self) -> u64 {
        self.device_features
    }

    fn set_driver_features(&mut self, features: u64) {
        self.driver_features = features;
    }
}

/// Reads a little-endian `u16`, answering zero past the end of `region`.
fn peek_u16(region: &[u8], at: usize) -> u16 {
    region
        .get(at..)
        .and_then(|rest| rest.first_chunk::<2>())
        .map_or(0, |chunk| u16::from_le_bytes(*chunk))
}
