// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The driver: bringing one block device up, and putting requests into
//! its queue.

use virtio_queue::{Buffer, Device, Queue, QueueMemory};

use crate::common::{self, Common};
use crate::config::{self, SECTOR_LEN};
use crate::error::BlkError;
use crate::features::{F_FLUSH, F_RO, WANTED};
use crate::registers::{Registers, Structure, Width, low_u8};
use crate::request::{self, Kind, Request};

/// Where the three areas of the request queue are, and how large the
/// caller made them.
///
/// The addresses are the device's view of memory, which the caller gets
/// from the object it mapped; nothing here derives an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rings {
    /// Physical address of the descriptor table.
    pub descriptor_table: u64,
    /// Physical address of the available ring.
    pub available_ring: u64,
    /// Physical address of the used ring.
    pub used_ring: u64,
    /// Descriptors the caller made room for. A device that offers fewer
    /// settles it; one that offers more is held to this.
    pub size: u16,
}

/// The message interrupt vectors of one device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Vectors {
    /// The vector a configuration change is reported on, or
    /// [`common::NO_VECTOR`].
    pub config: u16,
    /// The vector the request queue is reported on, or
    /// [`common::NO_VECTOR`].
    pub queue: u16,
}

impl Vectors {
    /// Neither a queue nor a configuration vector, which is a device
    /// driven without message interrupts.
    #[must_use]
    pub const fn none() -> Vectors {
        Vectors {
            config: common::NO_VECTOR,
            queue: common::NO_VECTOR,
        }
    }
}

/// One run of bytes the device reads or writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Segment {
    /// Physical address of the first byte.
    pub address: u64,
    /// How many bytes.
    pub length: u32,
}

/// Where the three parts of one request are.
///
/// The bytes are the caller's: it wrote the header with
/// [`Request::write_header`] and it reads the status byte back. What this
/// crate does is say which of the three the device reads and which it
/// writes, and in what order they go into the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chain {
    /// Physical address of the sixteen-byte header.
    pub header: u64,
    /// The data, for a read or a write; a flush has none.
    pub data: Option<Segment>,
    /// Physical address of the status byte.
    pub status: u64,
}

/// Bits of the interrupt status that say a queue was used
/// (virtio 4.1.4.5).
pub const ISR_QUEUE: u8 = 1 << 0;

/// Bits of the interrupt status that say the configuration changed.
pub const ISR_CONFIG: u8 = 1 << 1;

/// One block device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Blk {
    /// How far initialization has come.
    device: Device,
    /// The multiplier of the notification structure, from the capability
    /// the adapter read.
    notify_multiplier: u32,
    /// What was negotiated.
    features: u64,
    /// Sectors the device has.
    capacity: u64,
    /// Descriptors the request queue was configured with.
    queue_size: u16,
    /// Where in the notification structure this queue is notified.
    notify_offset: u16,
}

impl Blk {
    /// A device nothing has been done to. `notify_multiplier` is the
    /// value of the notification capability (virtio 4.1.4.4).
    #[must_use]
    pub const fn new(notify_multiplier: u32) -> Blk {
        Blk {
            device: Device::new(),
            notify_multiplier,
            features: 0,
            capacity: 0,
            queue_size: 0,
            notify_offset: 0,
        }
    }

    /// Writes zero to the device status, which resets the device.
    ///
    /// Virtio 4.1.4.3.2 has the driver wait for the status to read back
    /// as zero before it goes on. Waiting needs a clock, which no logic
    /// crate has, so the wait is the caller's and [`Blk::is_reset`] is
    /// what it waits on.
    pub fn reset(&mut self, registers: &mut impl Registers) {
        registers.write(Structure::Common, common::DEVICE_STATUS, Width::B8, 0);
        self.device = Device::new();
        self.features = 0;
        self.capacity = 0;
        self.queue_size = 0;
        self.notify_offset = 0;
    }

    /// Whether the device has finished the reset it was asked for.
    pub fn is_reset(&self, registers: &impl Registers) -> bool {
        self.status(registers) == 0
    }

    /// The device status byte.
    pub fn status(&self, registers: &impl Registers) -> u8 {
        low_u8(registers.read(Structure::Common, common::DEVICE_STATUS, Width::B8))
    }

    /// Brings the device up: steps 2 to 8 of virtio 3.1.1, with the
    /// queue of virtio 4.1.5.1.3 between the feature set and `DRIVER_OK`.
    ///
    /// The device must have been reset and have shown it: this refuses a
    /// status that is not zero rather than waiting for one.
    ///
    /// # Errors
    ///
    /// [`BlkError::NotReset`] for a device that is not at zero,
    /// [`BlkError::NoQueue`] for one with no request queue,
    /// [`BlkError::QueueSize`] for a size the caller's rings cannot hold,
    /// [`BlkError::NoVector`] for a vector the device would not take,
    /// [`BlkError::Generation`] for a configuration that will not stand
    /// still, and [`BlkError::Device`] for a step the state machine
    /// refused.
    pub fn initialize(
        &mut self,
        registers: &mut impl Registers,
        rings: &Rings,
        vectors: &Vectors,
    ) -> Result<(), BlkError> {
        if !rings.size.is_power_of_two() {
            return Err(BlkError::QueueSize(rings.size));
        }
        let status = self.status(registers);
        if status != 0 {
            return Err(BlkError::NotReset(status));
        }
        let mut common = Common::new(registers);
        self.device.acknowledge(&mut common)?;
        self.device.driver(&mut common)?;
        common.read_offered();
        let features = self.device.negotiate(&mut common, WANTED)?;
        let queue_size = configure_queue(&mut common, rings, vectors.queue)?;
        let notify_offset = notify_offset(&common, self.notify_multiplier)?;
        common.set_u16(common::CONFIG_MSIX_VECTOR, vectors.config);
        check_vector(&common, common::CONFIG_MSIX_VECTOR, vectors.config)?;
        common.set_u16(common::QUEUE_ENABLE, common::ENABLED);
        self.device.driver_ok(&mut common)?;
        // What was learned is kept only once every step has held, so a
        // refusal part way leaves the driver knowing nothing rather than
        // half of it.
        self.capacity = config::capacity(registers)?;
        self.features = features;
        self.queue_size = queue_size;
        self.notify_offset = notify_offset;
        Ok(())
    }

    /// Sectors the device has, each of [`SECTOR_LEN`] bytes.
    #[must_use]
    pub const fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Whether the device refuses to be written.
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.features & F_RO != 0
    }

    /// Whether the device takes a flush.
    #[must_use]
    pub const fn flushes(&self) -> bool {
        self.features & F_FLUSH != 0
    }

    /// What was negotiated.
    #[must_use]
    pub const fn features(&self) -> u64 {
        self.features
    }

    /// Descriptors the request queue was configured with.
    #[must_use]
    pub const fn queue_size(&self) -> u16 {
        self.queue_size
    }

    /// The state machine, which a queue operation needs.
    #[must_use]
    pub const fn device(&self) -> &Device {
        &self.device
    }

    /// Puts one request into the queue and answers the descriptor its
    /// chain begins at, which is what the used ring will name.
    ///
    /// The chain is the three parts of virtio 5.2.6 in the order that
    /// section gives them: the header the device reads, the data, and the
    /// status byte the device writes. What this refuses is a request the
    /// device would refuse or answer wrongly — data that is not whole
    /// sectors, a flush that carries data, a read or a write that does
    /// not, a request past the last sector, and a write to a device that
    /// said it is read-only.
    ///
    /// # Errors
    ///
    /// [`BlkError::Framing`], [`BlkError::DataLength`],
    /// [`BlkError::Capacity`], [`BlkError::ReadOnly`],
    /// [`BlkError::FlushSector`], and [`BlkError::Queue`] for what the
    /// queue refused.
    pub fn submit<const WORDS: usize>(
        &self,
        queue: &mut Queue<WORDS>,
        memory: &mut impl QueueMemory,
        request: &Request,
        chain: &Chain,
    ) -> Result<u16, BlkError> {
        if request.kind.changes_device() && self.is_read_only() {
            return Err(BlkError::ReadOnly);
        }
        if request.kind == Kind::Flush && request.sector != 0 {
            return Err(BlkError::FlushSector(request.sector));
        }
        let header = Buffer::to_device(chain.header, request::HEADER_LEN);
        let status = Buffer::from_device(chain.status, request::STATUS_LEN);
        let data = match (request.kind, chain.data) {
            (Kind::Flush, None) => None,
            (Kind::Flush, Some(_)) | (Kind::In | Kind::Out, None) => {
                return Err(BlkError::Framing);
            }
            (Kind::In | Kind::Out, Some(data)) => {
                self.check_data(request, data.length)?;
                Some(if request.kind.writes_data() {
                    Buffer::from_device(data.address, data.length)
                } else {
                    Buffer::to_device(data.address, data.length)
                })
            }
        };
        let head = match data {
            Some(data) => queue.add(&self.device, memory, &[header, data, status])?,
            None => queue.add(&self.device, memory, &[header, status])?,
        };
        Ok(head)
    }

    /// Tells the device that the queue has something in it.
    ///
    /// The address is the one virtio 4.1.4.4 derives, and what is written
    /// is the queue index, which is zero for the one queue a block device
    /// has without `VIRTIO_BLK_F_MQ`.
    pub fn notify(&self, registers: &mut impl Registers) {
        registers.write(
            Structure::Notify,
            self.notify_offset,
            Width::B16,
            u64::from(common::REQUEST_QUEUE),
        );
    }

    /// Reads the interrupt status, which the device clears as it is read
    /// (virtio 4.1.4.5). A device driven by message interrupts never
    /// sets it, so this is for one that is not.
    pub fn interrupt_status(&self, registers: &impl Registers) -> u8 {
        low_u8(registers.read(Structure::Isr, 0, Width::B8))
    }

    /// Notices a device that has asked for a reset.
    ///
    /// # Errors
    ///
    /// [`BlkError::Device`] with what the state machine made of the
    /// status.
    pub fn poll(&mut self, registers: &mut impl Registers) -> Result<(), BlkError> {
        let common = Common::new(registers);
        Ok(self.device.poll(&common)?)
    }

    /// That the data of a read or a write is whole sectors, and that the
    /// sectors it covers are on the device.
    fn check_data(&self, request: &Request, length: u32) -> Result<(), BlkError> {
        if length == 0 || !length.is_multiple_of(SECTOR_LEN) {
            return Err(BlkError::DataLength(length));
        }
        let sectors = u64::from(length / SECTOR_LEN);
        let end = request
            .sector
            .checked_add(sectors)
            .ok_or(BlkError::Capacity {
                sector: request.sector,
                sectors,
                capacity: self.capacity,
            })?;
        if end > self.capacity {
            return Err(BlkError::Capacity {
                sector: request.sector,
                sectors,
                capacity: self.capacity,
            });
        }
        Ok(())
    }
}

/// Configures the request queue and answers the size it was given.
fn configure_queue<R: Registers>(
    common: &mut Common<'_, R>,
    rings: &Rings,
    vector: u16,
) -> Result<u16, BlkError> {
    common.set_u16(common::QUEUE_SELECT, common::REQUEST_QUEUE);
    let offered = common.u16_at(common::QUEUE_SIZE);
    if offered == 0 {
        return Err(BlkError::NoQueue);
    }
    let size = offered.min(rings.size);
    if size == 0 || !size.is_power_of_two() {
        return Err(BlkError::QueueSize(size));
    }
    common.set_u16(common::QUEUE_SIZE, size);
    common.set_u16(common::QUEUE_MSIX_VECTOR, vector);
    check_vector(common, common::QUEUE_MSIX_VECTOR, vector)?;
    common.set_field(common::QUEUE_DESC, Width::B64, rings.descriptor_table);
    common.set_field(common::QUEUE_DRIVER, Width::B64, rings.available_ring);
    common.set_field(common::QUEUE_DEVICE, Width::B64, rings.used_ring);
    Ok(size)
}

/// Where the request queue is notified: the queue's own offset times the
/// multiplier of the capability (virtio 4.1.4.4).
///
/// The product is a byte offset into the notification structure, and this
/// crate reaches a structure through a sixteen-bit offset, so a product
/// that does not fit one is refused rather than truncated into a write
/// somewhere else in that structure.
fn notify_offset<R: Registers>(common: &Common<'_, R>, multiplier: u32) -> Result<u16, BlkError> {
    let offset = u32::from(common.u16_at(common::QUEUE_NOTIFY_OFF));
    let product = offset
        .checked_mul(multiplier)
        .ok_or(BlkError::NotifyOffset(offset))?;
    u16::try_from(product).map_err(|_| BlkError::NotifyOffset(offset))
}

/// That the device took the vector that was written.
///
/// Virtio 4.1.5.1.3, step 5, has the driver read the field back: the
/// value returns on success and `NO_VECTOR` when the device could not
/// take it.
fn check_vector<R: Registers>(
    common: &Common<'_, R>,
    offset: u16,
    wanted: u16,
) -> Result<(), BlkError> {
    if wanted == common::NO_VECTOR {
        return Ok(());
    }
    if common.u16_at(offset) == wanted {
        Ok(())
    } else {
        Err(BlkError::NoVector(wanted))
    }
}
