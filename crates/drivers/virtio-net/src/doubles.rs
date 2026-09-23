// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A device in memory: four structures of bytes that answer reads, record
//! every access, and can be scripted to behave like a device that refuses;
//! and a frame area in memory beside it.

use std::cell::{Cell, RefCell};
use std::vec;
use std::vec::Vec;

use crate::common;
use crate::frames::Frames;
use crate::net::{MAC, MAC_LEN};
use crate::registers::{Registers, Structure, Width};

/// One recorded access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// A field was read and answered the value.
    Read(Structure, u16, Width, u64),
    /// A field was written.
    Write(Structure, u16, Width, u64),
}

/// What a scripted device does that a well-behaved one does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Refusal {
    /// Nothing: the device behaves.
    #[default]
    None,
    /// It clears `FEATURES_OK` after the driver has set it, which
    /// virtio 3.1.1 has the driver read back and give up on.
    Features,
    /// It answers `NO_VECTOR` to every vector written, which is how
    /// virtio 4.1.5.1.3 says a device that has none left answers.
    Vectors,
    /// Its configuration generation changes under every read.
    Generation,
}

/// Bytes of the common configuration structure.
const COMMON_LEN: usize = 0x40;

/// Bytes of the device configuration structure.
const CONFIG_LEN: usize = 0x20;

/// Bytes of the notification structure.
const NOTIFY_LEN: usize = 0x10;

/// Queues a network device without `VIRTIO_NET_F_MQ` has.
const QUEUES: usize = 2;

/// A device whose structures are bytes in memory.
#[derive(Debug)]
pub struct RamDevice {
    /// The common configuration.
    common: RefCell<[u8; COMMON_LEN]>,
    /// The notification structure.
    notify: RefCell<[u8; NOTIFY_LEN]>,
    /// The interrupt status, which reading clears.
    isr: Cell<u8>,
    /// The device configuration.
    config: RefCell<[u8; CONFIG_LEN]>,
    /// The features the device offers.
    offered: u64,
    /// The features the driver wrote.
    accepted: Cell<u64>,
    /// Every access in order.
    log: RefCell<Vec<Access>>,
    /// What this device refuses.
    refusal: Refusal,
    /// How often the generation has been read, for [`Refusal::Generation`].
    generation: Cell<u8>,
    /// The size of each queue, which a reset gives back.
    offered_sizes: [u16; QUEUES],
    /// The size each queue stands at.
    sizes: RefCell<[u16; QUEUES]>,
}

impl RamDevice {
    /// A device offering `offered`, with both queues of `queue_size`
    /// descriptors and `address` as its hardware address.
    #[must_use]
    pub fn new(offered: u64, queue_size: u16, address: [u8; MAC_LEN]) -> RamDevice {
        Self::with_queues(offered, [queue_size; QUEUES], address)
    }

    /// The same, with a size per queue, so that a device missing one of
    /// them can be written down.
    #[must_use]
    pub fn with_queues(offered: u64, sizes: [u16; QUEUES], address: [u8; MAC_LEN]) -> RamDevice {
        let device = RamDevice {
            common: RefCell::new([0u8; COMMON_LEN]),
            notify: RefCell::new([0u8; NOTIFY_LEN]),
            isr: Cell::new(0),
            config: RefCell::new([0u8; CONFIG_LEN]),
            offered,
            accepted: Cell::new(0),
            log: RefCell::new(Vec::new()),
            refusal: Refusal::None,
            generation: Cell::new(0),
            offered_sizes: sizes,
            sizes: RefCell::new(sizes),
        };
        for (step, byte) in address.iter().enumerate() {
            let offset = MAC.saturating_add(u16::try_from(step).unwrap_or(0));
            Self::put(&device.config, offset, Width::B8, u64::from(*byte));
        }
        device
    }

    /// The same device, refusing what `refusal` names.
    #[must_use]
    pub const fn refusing(mut self, refusal: Refusal) -> RamDevice {
        self.refusal = refusal;
        self
    }

    /// The device status the driver has written.
    #[must_use]
    pub fn status(&self) -> u8 {
        u8::try_from(Self::take(&self.common, common::DEVICE_STATUS, Width::B8)).unwrap_or(0)
    }

    /// The features the driver accepted.
    #[must_use]
    pub const fn accepted(&self) -> u64 {
        self.accepted.get()
    }

    /// One field of the common configuration, without recording a read.
    #[must_use]
    pub fn common_field(&self, offset: u16, width: Width) -> u64 {
        Self::take(&self.common, offset, width)
    }

    /// The size queue `index` stands at.
    #[must_use]
    pub fn queue_size(&self, index: u16) -> u16 {
        self.sizes
            .borrow()
            .get(usize::from(index))
            .copied()
            .unwrap_or(0)
    }

    /// The interrupt status, which a read through the trait clears.
    pub fn set_interrupt_status(&self, value: u8) {
        self.isr.set(value);
    }

    /// Every access in order.
    #[must_use]
    pub fn log(&self) -> Vec<Access> {
        self.log.borrow().clone()
    }

    /// The writes in order, without the reads.
    #[must_use]
    pub fn writes(&self) -> Vec<(Structure, u16, u64)> {
        self.log
            .borrow()
            .iter()
            .filter_map(|access| match *access {
                Access::Write(structure, offset, _, value) => Some((structure, offset, value)),
                Access::Read(..) => None,
            })
            .collect()
    }

    /// Reads `width` bytes at `offset` of `bytes`.
    fn take<const N: usize>(bytes: &RefCell<[u8; N]>, offset: u16, width: Width) -> u64 {
        let at = usize::from(offset);
        let borrowed = bytes.borrow();
        let mut value = 0u64;
        for step in 0..usize::from(width.bytes()) {
            let byte = borrowed.get(at.saturating_add(step)).copied().unwrap_or(0);
            value |= u64::from(byte) << (step.saturating_mul(8));
        }
        value
    }

    /// Writes `width` bytes at `offset` of `bytes`.
    fn put<const N: usize>(bytes: &RefCell<[u8; N]>, offset: u16, width: Width, value: u64) {
        let at = usize::from(offset);
        let mut borrowed = bytes.borrow_mut();
        for step in 0..usize::from(width.bytes()) {
            if let Some(slot) = borrowed.get_mut(at.saturating_add(step)) {
                *slot = u8::try_from((value >> (step.saturating_mul(8))) & 0xFF).unwrap_or(0);
            }
        }
    }

    /// The queue the selector names.
    fn selected(&self) -> usize {
        usize::try_from(Self::take(&self.common, common::QUEUE_SELECT, Width::B16)).unwrap_or(0)
    }

    /// One window of thirty-two feature bits.
    fn feature_window(&self) -> u64 {
        let select = Self::take(&self.common, common::DEVICE_FEATURE_SELECT, Width::B32);
        match select {
            0 => self.offered & 0xFFFF_FFFF,
            1 => (self.offered >> 32) & 0xFFFF_FFFF,
            _ => 0,
        }
    }

    /// Takes the feature bits the driver wrote into the accepted set.
    fn accept(&self, value: u64) {
        let select = Self::take(&self.common, common::DRIVER_FEATURE_SELECT, Width::B32);
        let accepted = self.accepted.get();
        self.accepted.set(match select {
            0 => (accepted & !0xFFFF_FFFF) | (value & 0xFFFF_FFFF),
            1 => (accepted & 0xFFFF_FFFF) | ((value & 0xFFFF_FFFF) << 32),
            _ => accepted,
        });
    }

    /// What the device does with a status the driver wrote.
    fn wrote_status(&self, value: u64) {
        let status = u8::try_from(value & 0xFF).unwrap_or(0);
        if status == 0 {
            // Virtio 4.1.4.3: on reset the size field is the largest the
            // device supports again, whatever the last driver wrote.
            self.accepted.set(0);
            Self::put(&self.common, common::QUEUE_ENABLE, Width::B16, 0);
            *self.sizes.borrow_mut() = self.offered_sizes;
        }
        if self.refusal == Refusal::Features && status & virtio_queue::STATUS_FEATURES_OK != 0 {
            let cleared = u64::from(status & !virtio_queue::STATUS_FEATURES_OK);
            Self::put(&self.common, common::DEVICE_STATUS, Width::B8, cleared);
        }
    }
}

impl Registers for RamDevice {
    fn read(&self, structure: Structure, offset: u16, width: Width) -> u64 {
        let value = match (structure, offset) {
            (Structure::Common, common::DEVICE_FEATURE) => self.feature_window(),
            (Structure::Common, common::QUEUE_SIZE) => u64::from(
                self.sizes
                    .borrow()
                    .get(self.selected())
                    .copied()
                    .unwrap_or(0),
            ),
            // The notification offset of a queue is its own index, which
            // is what QEMU's device answers and what makes a wrong
            // multiplier visible as a write to the wrong queue.
            (Structure::Common, common::QUEUE_NOTIFY_OFF) => {
                u64::try_from(self.selected()).unwrap_or(0)
            }
            (Structure::Common, common::CONFIG_GENERATION) => {
                if self.refusal == Refusal::Generation {
                    self.generation.set(self.generation.get().wrapping_add(1));
                }
                u64::from(self.generation.get())
            }
            (Structure::Isr, _) => {
                let value = self.isr.get();
                self.isr.set(0);
                u64::from(value)
            }
            (Structure::Notify, _) => Self::take(&self.notify, offset, width),
            (Structure::Common, _) => Self::take(&self.common, offset, width),
            (Structure::Device, _) => Self::take(&self.config, offset, width),
        };
        self.log
            .borrow_mut()
            .push(Access::Read(structure, offset, width, value));
        width.mask(value)
    }

    fn write(&mut self, structure: Structure, offset: u16, width: Width, value: u64) {
        self.log
            .borrow_mut()
            .push(Access::Write(structure, offset, width, value));
        match (structure, offset) {
            (Structure::Common, common::DRIVER_FEATURE) => self.accept(value),
            (Structure::Common, common::QUEUE_SIZE) => {
                let selected = self.selected();
                if let Some(slot) = self.sizes.borrow_mut().get_mut(selected) {
                    *slot = u16::try_from(value & 0xFFFF).unwrap_or(0);
                }
            }
            (Structure::Common, common::CONFIG_MSIX_VECTOR | common::QUEUE_MSIX_VECTOR) => {
                let taken = if self.refusal == Refusal::Vectors {
                    u64::from(common::NO_VECTOR)
                } else {
                    value
                };
                Self::put(&self.common, offset, width, taken);
            }
            (Structure::Common, _) => {
                Self::put(&self.common, offset, width, value);
                if offset == common::DEVICE_STATUS {
                    self.wrote_status(value);
                }
            }
            (Structure::Notify, _) => Self::put(&self.notify, offset, width, value),
            (Structure::Isr, _) => self.isr.set(u8::try_from(value & 0xFF).unwrap_or(0)),
            (Structure::Device, _) => Self::put(&self.config, offset, width, value),
        }
    }
}

/// The frame buffers of one side, as one byte vector.
///
/// The address a buffer carries is its offset from `base`, which is what a
/// mapping of one contiguous physical range gives a driver (D-115).
#[derive(Clone, Debug)]
pub struct RamFrames {
    /// The bytes of every buffer, one after another.
    bytes: Vec<u8>,
    /// Bytes of one buffer.
    stride: u32,
    /// How many buffers.
    count: u16,
    /// The address the first buffer begins at.
    base: u64,
}

impl RamFrames {
    /// `count` buffers of `stride` bytes, beginning at `base`.
    #[must_use]
    pub fn new(count: u16, stride: u32, base: u64) -> RamFrames {
        let len = usize::from(count).saturating_mul(usize::try_from(stride).unwrap_or(0));
        RamFrames {
            bytes: vec![0; len],
            stride,
            count,
            base,
        }
    }

    /// Where buffer `index` begins in the bytes.
    fn at(&self, index: u16) -> Option<usize> {
        if index >= self.count {
            return None;
        }
        usize::from(index).checked_mul(usize::try_from(self.stride).ok()?)
    }

    /// Acts as the device: writes `frame` into buffer `index` behind a
    /// header of `header` bytes, and answers what a used element would
    /// report as written.
    ///
    /// A device writes into the buffer it was given and no further, so
    /// what is longer than one buffer ends at that buffer's last byte.
    pub fn deliver(&mut self, index: u16, header: &[u8], frame: &[u8]) -> u32 {
        let Some(at) = self.at(index) else {
            return 0;
        };
        let room = usize::try_from(self.stride).unwrap_or(0);
        let mut written = 0usize;
        for (step, byte) in header.iter().chain(frame.iter()).take(room).enumerate() {
            if let Some(slot) = self.bytes.get_mut(at.saturating_add(step)) {
                *slot = *byte;
                written = written.saturating_add(1);
            }
        }
        u32::try_from(written).unwrap_or(0)
    }

    /// What buffer `index` holds, for a test that reads what was written
    /// into it.
    #[must_use]
    pub fn buffer(&self, index: u16) -> &[u8] {
        self.at(index)
            .and_then(|at| {
                self.bytes
                    .get(at..at.saturating_add(usize::try_from(self.stride).ok()?))
            })
            .unwrap_or(&[])
    }
}

impl Frames for RamFrames {
    fn count(&self) -> u16 {
        self.count
    }

    fn len(&self) -> u32 {
        self.stride
    }

    fn address(&self, index: u16) -> Option<u64> {
        let at = self.at(index)?;
        self.base.checked_add(u64::try_from(at).ok()?)
    }

    fn copy(&self, index: u16, offset: usize, out: &mut [u8]) -> Option<()> {
        let at = self.at(index)?;
        let end = offset.checked_add(out.len())?;
        if end > usize::try_from(self.stride).ok()? {
            return None;
        }
        out.copy_from_slice(
            self.bytes
                .get(at.checked_add(offset)?..at.checked_add(end)?)?,
        );
        Some(())
    }

    fn bytes_mut(&mut self, index: u16) -> Option<&mut [u8]> {
        let at = self.at(index)?;
        let end = at.checked_add(usize::try_from(self.stride).ok()?)?;
        self.bytes.get_mut(at..end)
    }
}
