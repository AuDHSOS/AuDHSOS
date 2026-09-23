// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The driver: bringing one network device up.
//!
//! The sequence is steps 2 to 8 of virtio 3.1.1, with the two queues of
//! virtio 4.1.5.1.3 between the feature set and `DRIVER_OK`, over the
//! state machine `virtio-queue` already has.

use virtio_queue::Device;

use crate::common::{self, Common};
use crate::error::NetError;
use crate::features::{F_MAC, WANTED};
use crate::net::{MAC_LEN, mac};
use crate::queues::{Configured, Queues, Rings, Side, Vectors};
use crate::registers::{Registers, Structure, Width, low_u8};

/// The entry of a descriptor table that names no buffer of this driver.
/// A queue holds at most 32768 descriptors, so this is no descriptor.
pub const NO_BUFFER: u16 = u16::MAX;

/// Bits of the interrupt status that say a queue was used
/// (virtio 4.1.4.5).
pub const ISR_QUEUE: u8 = 1 << 0;

/// Bits of the interrupt status that say the configuration changed.
pub const ISR_CONFIG: u8 = 1 << 1;

/// One network device.
///
/// `SLOTS` is how many descriptors of either queue and how many buffers of
/// either area this driver keeps track of. It bounds four tables: which
/// buffer a descriptor was handed out for, on each side, and which buffer
/// is with the device, on each side. A queue or an area larger than this
/// is refused rather than driven in part.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Net<const SLOTS: usize> {
    /// How far initialization has come.
    pub(crate) device: Device,
    /// The multiplier of the notification structure, from the capability
    /// the adapter read.
    pub(crate) notify_multiplier: u32,
    /// What was negotiated.
    pub(crate) features: u64,
    /// The hardware address, for a device that offered
    /// `VIRTIO_NET_F_MAC`.
    pub(crate) mac: Option<[u8; MAC_LEN]>,
    /// What the receive queue was configured with.
    pub(crate) receive: Configured,
    /// What the transmit queue was configured with.
    pub(crate) transmit: Configured,
    /// Which receive buffer each descriptor of the receive queue was
    /// handed out for, or [`NO_BUFFER`].
    pub(crate) taken: [u16; SLOTS],
    /// Which receive buffers are with the device.
    pub(crate) posted: [bool; SLOTS],
    /// The same for the transmit queue.
    pub(crate) sending: [u16; SLOTS],
    /// Which transmit buffers are with the device.
    pub(crate) busy: [bool; SLOTS],
    /// The first transmit buffer checked after a completion.
    pub(crate) free_hint: u16,
}

impl<const SLOTS: usize> Net<SLOTS> {
    /// A device nothing has been done to. `notify_multiplier` is the
    /// value of the notification capability (virtio 4.1.4.4).
    #[must_use]
    pub const fn new(notify_multiplier: u32) -> Net<SLOTS> {
        Net {
            device: Device::new(),
            notify_multiplier,
            features: 0,
            mac: None,
            receive: Configured { size: 0, notify: 0 },
            transmit: Configured { size: 0, notify: 0 },
            taken: [NO_BUFFER; SLOTS],
            posted: [false; SLOTS],
            sending: [NO_BUFFER; SLOTS],
            busy: [false; SLOTS],
            free_hint: 0,
        }
    }

    /// Writes zero to the device status, which resets the device.
    ///
    /// Virtio 4.1.4.3.2 has the driver wait for the status to read back
    /// as zero before it goes on. Waiting needs a clock, which no logic
    /// crate has, so the wait is the caller's and [`Net::is_reset`] is
    /// what it waits on.
    pub fn reset(&mut self, registers: &mut impl Registers) {
        registers.write(Structure::Common, common::DEVICE_STATUS, Width::B8, 0);
        self.device = Device::new();
        self.features = 0;
        self.mac = None;
        self.receive = Configured { size: 0, notify: 0 };
        self.transmit = Configured { size: 0, notify: 0 };
        self.taken = [NO_BUFFER; SLOTS];
        self.posted = [false; SLOTS];
        self.sending = [NO_BUFFER; SLOTS];
        self.busy = [false; SLOTS];
        self.free_hint = 0;
    }

    /// Whether the device has finished the reset it was asked for.
    pub fn is_reset(&self, registers: &impl Registers) -> bool {
        self.status(registers) == 0
    }

    /// The device status byte.
    pub fn status(&self, registers: &impl Registers) -> u8 {
        low_u8(registers.read(Structure::Common, common::DEVICE_STATUS, Width::B8))
    }

    /// Brings the device up and leaves both queues enabled.
    ///
    /// The device must have been reset and have shown it: this refuses a
    /// status that is not zero rather than waiting for one. The receive
    /// queue holds no buffer yet; [`Net::fill`] is what puts them there.
    ///
    /// # Errors
    ///
    /// [`NetError::NotReset`] for a device that is not at zero,
    /// [`NetError::NoQueue`] for one that lacks either queue,
    /// [`NetError::QueueSize`] for a size the caller's rings cannot hold,
    /// [`NetError::Slots`] for one larger than this driver's tables,
    /// [`NetError::NoVector`] for a vector the device would not take,
    /// [`NetError::NotifyOffset`] for a notification offset that does not
    /// fit, [`NetError::Generation`] for a configuration that will not
    /// stand still, and [`NetError::Device`] for a step the state machine
    /// refused.
    pub fn initialize(
        &mut self,
        registers: &mut impl Registers,
        queues: &Queues,
        vectors: &Vectors,
    ) -> Result<(), NetError> {
        for side in Side::ALL {
            // Zero is not a power of two, so one test covers both the
            // size a caller left out and the size it got wrong.
            let size = queues.of(side).size;
            if !size.is_power_of_two() {
                return Err(NetError::QueueSize(size));
            }
        }
        let status = self.status(registers);
        if status != 0 {
            return Err(NetError::NotReset(status));
        }
        let mut common = Common::new(registers);
        self.device.acknowledge(&mut common)?;
        self.device.driver(&mut common)?;
        common.read_offered();
        let features = self.device.negotiate(&mut common, WANTED)?;
        let mut configured = [Configured::default(); 2];
        for (slot, side) in configured.iter_mut().zip(Side::ALL) {
            *slot = configure_queue(
                &mut common,
                side,
                queues.of(side),
                vectors.of(side),
                self.notify_multiplier,
            )?;
            if usize::from(slot.size) > SLOTS {
                return Err(NetError::Slots(slot.size));
            }
        }
        common.set_u16(common::CONFIG_MSIX_VECTOR, vectors.config);
        check_vector(&common, common::CONFIG_MSIX_VECTOR, vectors.config)?;
        self.device.driver_ok(&mut common)?;
        // What was learned is kept only once every step has held, so a
        // refusal part way leaves the driver knowing nothing rather than
        // half of it.
        let address = if features & F_MAC == 0 {
            None
        } else {
            Some(mac(registers)?)
        };
        let [receive, transmit] = configured;
        self.mac = address;
        self.features = features;
        self.receive = receive;
        self.transmit = transmit;
        Ok(())
    }

    /// The hardware address the device carries, or `None` for a device
    /// that did not offer `VIRTIO_NET_F_MAC`.
    ///
    /// Six zeroes are a hardware address a device may carry, so a device
    /// that named none answers nothing rather than six of them.
    #[must_use]
    pub const fn mac(&self) -> Option<[u8; MAC_LEN]> {
        self.mac
    }

    /// What was negotiated.
    #[must_use]
    pub const fn features(&self) -> u64 {
        self.features
    }

    /// What one queue was configured with.
    #[must_use]
    pub const fn queue(&self, side: Side) -> Configured {
        match side {
            Side::Receive => self.receive,
            Side::Transmit => self.transmit,
        }
    }

    /// The state machine, which a queue operation needs.
    #[must_use]
    pub const fn device(&self) -> &Device {
        &self.device
    }

    /// Tells the device that one queue has something in it.
    ///
    /// The offset is the one virtio 4.1.4.4 derives, and what is written
    /// is the queue index.
    pub fn notify(&self, registers: &mut impl Registers, side: Side) {
        registers.write(
            Structure::Notify,
            self.queue(side).notify,
            Width::B16,
            u64::from(side.index()),
        );
    }

    /// Reads the interrupt status, which the device clears as it is read
    /// (virtio 4.1.4.5). A device driven by message interrupts never sets
    /// it, so this is for one that is not.
    pub fn interrupt_status(&self, registers: &impl Registers) -> u8 {
        low_u8(registers.read(Structure::Isr, 0, Width::B8))
    }

    /// Whether the device has to be told that `queue` has something in
    /// it, which is the flag of virtio 2.7.10 the device writes into the
    /// used ring.
    ///
    /// # Errors
    ///
    /// [`NetError::Queue`] before `DRIVER_OK`, and for a used ring that
    /// cannot hold its flags.
    pub fn should_notify<const WORDS: usize>(
        &self,
        queue: &virtio_queue::Queue<WORDS>,
        memory: &impl virtio_queue::QueueMemory,
    ) -> Result<bool, NetError> {
        Ok(queue.should_notify(&self.device, memory)?)
    }

    /// Notices a device that has asked for a reset.
    ///
    /// # Errors
    ///
    /// [`NetError::Device`] with what the state machine made of the
    /// status.
    pub fn poll(&mut self, registers: &mut impl Registers) -> Result<(), NetError> {
        let common = Common::new(registers);
        Ok(self.device.poll(&common)?)
    }
}

/// Configures one queue and answers the size and the notification offset
/// it was given.
fn configure_queue<R: Registers>(
    common: &mut Common<'_, R>,
    side: Side,
    rings: &Rings,
    vector: u16,
    multiplier: u32,
) -> Result<Configured, NetError> {
    common.set_u16(common::QUEUE_SELECT, side.index());
    let offered = common.u16_at(common::QUEUE_SIZE);
    if offered == 0 {
        return Err(NetError::NoQueue(side));
    }
    let descriptors = offered.min(rings.size);
    if !descriptors.is_power_of_two() {
        return Err(NetError::QueueSize(descriptors));
    }
    common.set_u16(common::QUEUE_SIZE, descriptors);
    common.set_u16(common::QUEUE_MSIX_VECTOR, vector);
    check_vector(common, common::QUEUE_MSIX_VECTOR, vector)?;
    common.set_field(common::QUEUE_DESC, Width::B64, rings.descriptor_table);
    common.set_field(common::QUEUE_DRIVER, Width::B64, rings.available_ring);
    common.set_field(common::QUEUE_DEVICE, Width::B64, rings.used_ring);
    let notify = notify_offset(common, multiplier)?;
    common.set_u16(common::QUEUE_ENABLE, common::ENABLED);
    Ok(Configured {
        size: descriptors,
        notify,
    })
}

/// Where the selected queue is notified: the queue's own offset times the
/// multiplier of the capability (virtio 4.1.4.4).
///
/// The product is a byte offset into the notification structure, and this
/// crate reaches a structure through a sixteen-bit offset, so a product
/// that does not fit one is refused rather than truncated into a write
/// somewhere else in that structure.
fn notify_offset<R: Registers>(common: &Common<'_, R>, multiplier: u32) -> Result<u16, NetError> {
    let offset = u32::from(common.u16_at(common::QUEUE_NOTIFY_OFF));
    let product = offset
        .checked_mul(multiplier)
        .ok_or(NetError::NotifyOffset(offset))?;
    u16::try_from(product).map_err(|_| NetError::NotifyOffset(offset))
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
) -> Result<(), NetError> {
    if wanted == common::NO_VECTOR {
        return Ok(());
    }
    if common.u16_at(offset) == wanted {
        Ok(())
    } else {
        Err(NetError::NoVector(wanted))
    }
}
