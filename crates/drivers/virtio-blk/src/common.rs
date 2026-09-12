// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The common configuration structure of virtio 4.1.4.3: where each field
//! lies, and the bridge that makes it the `DeviceRegisters` the state
//! machine of `virtio-queue` reads through.

use virtio_queue::DeviceRegisters;

use crate::registers::{Registers, Structure, Width, low_u8, low_u16};

/// Which window of thirty-two feature bits `DEVICE_FEATURE` shows.
pub const DEVICE_FEATURE_SELECT: u16 = 0x00;

/// The feature bits the device offers, in the selected window.
pub const DEVICE_FEATURE: u16 = 0x04;

/// Which window of thirty-two feature bits `DRIVER_FEATURE` writes.
pub const DRIVER_FEATURE_SELECT: u16 = 0x08;

/// The feature bits the driver accepts, in the selected window.
pub const DRIVER_FEATURE: u16 = 0x0C;

/// The interrupt vector of a configuration change.
pub const CONFIG_MSIX_VECTOR: u16 = 0x10;

/// How many queues the device has.
pub const NUM_QUEUES: u16 = 0x12;

/// The device status byte of virtio 2.1.
pub const DEVICE_STATUS: u16 = 0x14;

/// The generation of the device configuration space (virtio 2.5).
pub const CONFIG_GENERATION: u16 = 0x15;

/// Which queue the fields below refer to.
pub const QUEUE_SELECT: u16 = 0x16;

/// How many descriptors the selected queue has.
pub const QUEUE_SIZE: u16 = 0x18;

/// The interrupt vector of the selected queue.
pub const QUEUE_MSIX_VECTOR: u16 = 0x1A;

/// Whether the device may take buffers from the selected queue.
pub const QUEUE_ENABLE: u16 = 0x1C;

/// The notification offset of the selected queue, before the multiplier.
pub const QUEUE_NOTIFY_OFF: u16 = 0x1E;

/// The physical address of the descriptor table of the selected queue.
pub const QUEUE_DESC: u16 = 0x20;

/// The physical address of the available ring of the selected queue.
pub const QUEUE_DRIVER: u16 = 0x28;

/// The physical address of the used ring of the selected queue.
pub const QUEUE_DEVICE: u16 = 0x30;

/// Bytes of the structure this driver reaches into.
pub const LEN: u16 = 0x38;

/// The vector value that means no interrupt is mapped (virtio 4.1.5.1.3).
pub const NO_VECTOR: u16 = 0xFFFF;

/// The value that enables a queue. Virtio 4.1.4.3.2 forbids writing zero.
pub const ENABLED: u16 = 1;

/// The request queue of a block device, which is queue zero. Without
/// `VIRTIO_BLK_F_MQ` it is the only one (virtio 5.2.2).
pub const REQUEST_QUEUE: u16 = 0;

/// The common configuration of one device.
pub struct Common<'a, R: Registers> {
    /// The registers of the device.
    registers: &'a mut R,
    /// What the device offered, as [`Common::read_offered`] last read it.
    offered: u64,
}

impl<'a, R: Registers> Common<'a, R> {
    /// The common configuration reached through `registers`. Nothing is
    /// read or written until something is asked for.
    pub const fn new(registers: &'a mut R) -> Common<'a, R> {
        Common {
            registers,
            offered: 0,
        }
    }

    /// Reads the features the device offers, and answers them.
    ///
    /// The feature bits come through a window of thirty-two at a time, so
    /// reading all sixty-four is two writes of a selector and two reads,
    /// while [`DeviceRegisters::device_features`] is a shared method that
    /// cannot write. So the windowing happens here, where the registers
    /// are held exclusively, and the answer is kept for that method.
    ///
    /// This belongs where virtio 3.1.1 puts it, which is step 4, after
    /// `DRIVER` is set: writing the selector is a write to the device,
    /// and the driver makes none before it has said it is there.
    pub fn read_offered(&mut self) -> u64 {
        let low = window(self.registers, 0);
        let high = window(self.registers, 1);
        self.offered = u64::from(low) | (u64::from(high) << 32);
        self.offered
    }

    /// Reads one field of the structure.
    #[must_use]
    pub fn field(&self, offset: u16, width: Width) -> u64 {
        self.registers.read(Structure::Common, offset, width)
    }

    /// Writes one field of the structure.
    pub fn set_field(&mut self, offset: u16, width: Width, value: u64) {
        self.registers
            .write(Structure::Common, offset, width, value);
    }

    /// Reads one sixteen-bit field.
    #[must_use]
    pub fn u16_at(&self, offset: u16) -> u16 {
        low_u16(self.field(offset, Width::B16))
    }

    /// Writes one sixteen-bit field.
    pub fn set_u16(&mut self, offset: u16, value: u16) {
        self.set_field(offset, Width::B16, u64::from(value));
    }

    /// Reads one eight-bit field.
    #[must_use]
    pub fn u8_at(&self, offset: u16) -> u8 {
        low_u8(self.field(offset, Width::B8))
    }

    /// The features the device offered.
    #[must_use]
    pub const fn offered(&self) -> u64 {
        self.offered
    }

    /// The registers this value reaches the device through, for a caller
    /// that has to touch another structure.
    pub const fn registers(&mut self) -> &mut R {
        self.registers
    }
}

impl<R: Registers> DeviceRegisters for Common<'_, R> {
    fn status(&self) -> u8 {
        self.u8_at(DEVICE_STATUS)
    }

    fn set_status(&mut self, status: u8) {
        self.set_field(DEVICE_STATUS, Width::B8, u64::from(status));
    }

    /// What [`Common::read_offered`] read, and zero before it has been
    /// called. Zero is refused by the state machine as a legacy device,
    /// which is the safe way for that mistake to end.
    fn device_features(&self) -> u64 {
        self.offered
    }

    fn set_driver_features(&mut self, features: u64) {
        let low = features & 0xFFFF_FFFF;
        let high = (features >> 32) & 0xFFFF_FFFF;
        self.set_field(DRIVER_FEATURE_SELECT, Width::B32, 0);
        self.set_field(DRIVER_FEATURE, Width::B32, low);
        self.set_field(DRIVER_FEATURE_SELECT, Width::B32, 1);
        self.set_field(DRIVER_FEATURE, Width::B32, high);
    }
}

/// One window of thirty-two feature bits the device offers.
fn window(registers: &mut impl Registers, select: u32) -> u32 {
    registers.write(
        Structure::Common,
        DEVICE_FEATURE_SELECT,
        Width::B32,
        u64::from(select),
    );
    let value = registers.read(Structure::Common, DEVICE_FEATURE, Width::B32);
    u32::try_from(value & 0xFFFF_FFFF).unwrap_or(0)
}
