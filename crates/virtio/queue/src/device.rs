// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The device initialization state machine of virtio 1.x.
//!
//! The status bits are section 2.1 of virtio 1.4, the sequence is
//! section 3.1.1, and the feature bits are section 6.
//!
//! Invariants: a step is taken only from the phase before it; a device
//! that asks for a reset, and a device that rejects the feature set, put
//! the machine into [`Phase::Failed`], which every step but
//! [`Device::reset`] refuses.

use crate::error::DeviceError;

/// Status bit: the driver has noticed the device (section 2.1).
pub const STATUS_ACKNOWLEDGE: u8 = 1 << 0;

/// Status bit: the driver knows how to drive the device.
pub const STATUS_DRIVER: u8 = 1 << 1;

/// Status bit: the driver is set up and the queues may be used.
pub const STATUS_DRIVER_OK: u8 = 1 << 2;

/// Status bit: the driver has settled the feature set.
pub const STATUS_FEATURES_OK: u8 = 1 << 3;

/// Status bit: the device has hit an error and wants a reset. The device
/// sets it; the driver only reads it. Section 2.1.2 has a device set it
/// when it enters a state a reset is needed for, and section 2.1.1 has
/// the driver not rely on any operation completing once it is set.
pub const STATUS_DEVICE_NEEDS_RESET: u8 = 1 << 6;

/// Status bit: the driver has given up on the device.
pub const STATUS_FAILED: u8 = 1 << 7;

/// Feature bit: a buffer may hold a table of further descriptors. Not
/// implemented (D-52).
pub const F_INDIRECT_DESC: u64 = 1 << 28;

/// Feature bit: the two event fields suppress notifications by index
/// instead of by flag. Not implemented (D-52).
pub const F_EVENT_IDX: u64 = 1 << 29;

/// Feature bit: the device is a virtio 1.x device rather than a legacy
/// one. This crate drives nothing without it.
pub const F_VERSION_1: u64 = 1 << 32;

/// Feature bit: the packed ring layout instead of the split one. Not
/// implemented (D-52).
pub const F_RING_PACKED: u64 = 1 << 34;

/// The features this crate refuses to negotiate (D-52). Each would change
/// the layout or the notification rule of the ring this crate implements,
/// so a driver that asks for one learns it here rather than at the first
/// descriptor.
pub const UNSUPPORTED: u64 = F_INDIRECT_DESC | F_EVENT_IDX | F_RING_PACKED;

/// How far the initialization has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// The status register has been written zero, or nothing has been
    /// written yet.
    Reset,
    /// `ACKNOWLEDGE` is set.
    Acknowledged,
    /// `DRIVER` is set.
    Driven,
    /// `FEATURES_OK` is set and the device still shows it.
    FeaturesOk,
    /// `DRIVER_OK` is set: the queues may be used.
    Live,
    /// The device is unusable. The reason is the one every further step
    /// returns; only [`Device::reset`] leaves this phase.
    Failed(DeviceError),
}

/// The status and feature registers of one device.
///
/// Virtio 1.x offers the feature bits through a window of thirty-two at a
/// time, and the window is worked differently by the PCI transport and by
/// the MMIO one. Selecting the window is the adapter's business, so this
/// trait asks for all sixty-four at once.
pub trait DeviceRegisters {
    /// The device status register.
    fn status(&self) -> u8;

    /// Writes the device status register. Writing zero resets the device.
    fn set_status(&mut self, status: u8);

    /// The features the device offers.
    fn device_features(&self) -> u64;

    /// Writes the features the driver accepts.
    fn set_driver_features(&mut self, features: u64);
}

/// The initialization state machine of one device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Device {
    /// How far initialization has come.
    phase: Phase,
    /// What [`Device::negotiate`] settled on.
    features: u64,
}

impl Device {
    /// A device that has not been touched.
    #[must_use]
    pub const fn new() -> Device {
        Device {
            phase: Phase::Reset,
            features: 0,
        }
    }

    /// How far initialization has come.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether the device is live: `DRIVER_OK` is set, so the device may
    /// consume buffers and the driver may notify it.
    ///
    /// Section 3.1.1 forbids the driver to send a buffer available
    /// notification before this, and section 2.1.2 forbids the device to
    /// consume a buffer or send a used buffer notification before it.
    #[must_use]
    pub const fn is_live(&self) -> bool {
        matches!(self.phase, Phase::Live)
    }

    /// Whether buffers may go into a queue.
    ///
    /// This is true one step earlier than [`Device::is_live`], and the
    /// difference is not a nicety. Step 7 of section 3.1.1 is the
    /// device-specific setup, and *population of virtqueues* is named in
    /// it; step 8 sets `DRIVER_OK`. A receive queue is filled in step 7
    /// and the device begins to consume it in step 8, so a queue that
    /// refused a buffer until `DRIVER_OK` would refuse the one thing
    /// that step exists for.
    ///
    /// What stays behind `DRIVER_OK` is the notification, which section
    /// 3.1.1 forbids in as many words, and reading the used ring, which
    /// the device may not have written yet.
    #[must_use]
    pub const fn may_populate_queues(&self) -> bool {
        matches!(self.phase, Phase::FeaturesOk | Phase::Live)
    }

    /// The feature set [`Device::negotiate`] settled on, or zero before
    /// it ran.
    #[must_use]
    pub const fn features(&self) -> u64 {
        self.features
    }

    /// Resets the device and forgets everything about it.
    ///
    /// This is the one step that is taken from any phase, because it is
    /// the way out of [`Phase::Failed`].
    ///
    /// A transport may require the driver to wait until the status reads
    /// back as zero before it goes on. Waiting needs a bound and a notion
    /// of time that this crate has neither of, so it is the adapter's,
    /// and this call returns as soon as the write is done.
    pub fn reset(&mut self, registers: &mut impl DeviceRegisters) {
        registers.set_status(0);
        self.phase = Phase::Reset;
        self.features = 0;
    }

    /// Sets `ACKNOWLEDGE`, which is step 2 of section 3.1.1.
    ///
    /// # Errors
    ///
    /// [`DeviceError::OutOfOrder`] unless the device was reset, and the
    /// failure reason when it has failed.
    pub fn acknowledge(&mut self, registers: &mut impl DeviceRegisters) -> Result<(), DeviceError> {
        self.expect(registers, Phase::Reset)?;
        self.advance(registers, STATUS_ACKNOWLEDGE, Phase::Acknowledged);
        Ok(())
    }

    /// Sets `DRIVER`, which is step 3 of section 3.1.1.
    ///
    /// # Errors
    ///
    /// [`DeviceError::OutOfOrder`] unless `ACKNOWLEDGE` is set, and the
    /// failure reason when the device has failed.
    pub fn driver(&mut self, registers: &mut impl DeviceRegisters) -> Result<(), DeviceError> {
        self.expect(registers, Phase::Acknowledged)?;
        self.advance(registers, STATUS_DRIVER, Phase::Driven);
        Ok(())
    }

    /// Settles the feature set and sets `FEATURES_OK`, which is steps 4
    /// to 6 of section 3.1.1: read what the device offers, write the
    /// subset, set the bit, and read the status back to see the device
    /// still showing it.
    ///
    /// `wanted` is what the driver would take. What comes back is what
    /// the device also offers, plus [`F_VERSION_1`], which is required.
    /// A feature the driver wanted and the device does not offer is not
    /// an error here: section 2.2.1 has the driver either go into
    /// backwards compatibility mode or fail, and which of the two it is
    /// is the driver's judgement, so the answer is in the returned set.
    /// What that section forbids is accepting a feature the device did
    /// not offer, and the returned set is an intersection, so it cannot.
    ///
    /// The same section forbids accepting a feature that requires
    /// another one that was not accepted. Which features require which
    /// is a property of the device type, which this crate does not know,
    /// so that rule is the caller's to keep when it builds `wanted`.
    ///
    /// # Errors
    ///
    /// [`DeviceError::UnsupportedFeature`] when `wanted` names one of
    /// [`UNSUPPORTED`], and nothing is written. [`DeviceError::Legacy`]
    /// when the device does not offer [`F_VERSION_1`], and
    /// [`DeviceError::FeaturesRejected`] when it clears `FEATURES_OK`
    /// after the driver set it; both fail the device.
    pub fn negotiate(
        &mut self,
        registers: &mut impl DeviceRegisters,
        wanted: u64,
    ) -> Result<u64, DeviceError> {
        self.expect(registers, Phase::Driven)?;
        let refused = wanted & UNSUPPORTED;
        if refused != 0 {
            return Err(DeviceError::UnsupportedFeature(refused));
        }
        let offered = registers.device_features();
        if offered & F_VERSION_1 == 0 {
            return Err(self.fail(registers, DeviceError::Legacy));
        }
        let accepted = (wanted & offered) | F_VERSION_1;
        registers.set_driver_features(accepted);
        self.advance(registers, STATUS_FEATURES_OK, Phase::FeaturesOk);
        if registers.status() & STATUS_FEATURES_OK == 0 {
            return Err(self.fail(registers, DeviceError::FeaturesRejected));
        }
        self.features = accepted;
        Ok(accepted)
    }

    /// Sets `DRIVER_OK`, which is step 8 of section 3.1.1. From here the
    /// device is live and may consume buffers, which section 2.1.2
    /// forbids it before this point.
    ///
    /// # Errors
    ///
    /// [`DeviceError::OutOfOrder`] unless `FEATURES_OK` is set, and the
    /// failure reason when the device has failed.
    pub fn driver_ok(&mut self, registers: &mut impl DeviceRegisters) -> Result<(), DeviceError> {
        self.expect(registers, Phase::FeaturesOk)?;
        self.advance(registers, STATUS_DRIVER_OK, Phase::Live);
        Ok(())
    }

    /// Reads the status register and notices a device that asks for a
    /// reset.
    ///
    /// A queue operation sees no register, so this is how a driver
    /// notices `DEVICE_NEEDS_RESET` while the device is live.
    ///
    /// # Errors
    ///
    /// [`DeviceError::NeedsReset`] when the device asks for one, and the
    /// failure reason when it has already failed.
    pub fn poll(&mut self, registers: &impl DeviceRegisters) -> Result<(), DeviceError> {
        self.alive(registers)
    }

    /// Refuses a step when the device has failed or asks for a reset.
    fn alive(&mut self, registers: &impl DeviceRegisters) -> Result<(), DeviceError> {
        if let Phase::Failed(reason) = self.phase {
            return Err(reason);
        }
        if registers.status() & STATUS_DEVICE_NEEDS_RESET != 0 {
            self.phase = Phase::Failed(DeviceError::NeedsReset);
            return Err(DeviceError::NeedsReset);
        }
        Ok(())
    }

    /// Refuses a step that does not follow the phase the device is in.
    fn expect(
        &mut self,
        registers: &impl DeviceRegisters,
        wanted: Phase,
    ) -> Result<(), DeviceError> {
        self.alive(registers)?;
        if self.phase == wanted {
            Ok(())
        } else {
            Err(DeviceError::OutOfOrder)
        }
    }

    /// Adds `bit` to the status register and moves to `next`.
    ///
    /// Adds: section 2.1.1 forbids the driver to clear a status bit, so
    /// every step is an or of the value the device is showing.
    fn advance(&mut self, registers: &mut impl DeviceRegisters, bit: u8, next: Phase) {
        let status = registers.status() | bit;
        registers.set_status(status);
        self.phase = next;
    }

    /// Writes `FAILED`, records `reason`, and hands it back.
    ///
    /// Section 3.1.1 has the driver set `FAILED` when a step goes
    /// irrecoverably wrong and stop, and section 2.1.1 has it reset the
    /// device before trying again, which is why [`Phase::Failed`] leaves
    /// only [`Device::reset`] open.
    fn fail(&mut self, registers: &mut impl DeviceRegisters, reason: DeviceError) -> DeviceError {
        let status = registers.status() | STATUS_FAILED;
        registers.set_status(status);
        self.phase = Phase::Failed(reason);
        reason
    }
}

impl Default for Device {
    fn default() -> Device {
        Device::new()
    }
}
