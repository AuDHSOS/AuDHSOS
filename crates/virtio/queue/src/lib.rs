// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(any(test, feature = "test-doubles")), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod descriptor;
pub mod device;
#[cfg(any(test, feature = "test-doubles"))]
pub mod doubles;
pub mod error;
pub mod memory;
pub mod queue;

pub use descriptor::{Buffer, DESC_F_INDIRECT, DESC_F_NEXT, DESC_F_WRITE, Descriptor, Direction};
pub use device::{
    Device, DeviceRegisters, F_EVENT_IDX, F_INDIRECT_DESC, F_RING_PACKED, F_VERSION_1, Phase,
    STATUS_ACKNOWLEDGE, STATUS_DEVICE_NEEDS_RESET, STATUS_DRIVER, STATUS_DRIVER_OK, STATUS_FAILED,
    STATUS_FEATURES_OK, UNSUPPORTED,
};
pub use error::{Area, DeviceError, QueueError};
pub use memory::{MAX_QUEUE_SIZE, QueueMemory};
pub use queue::{AVAIL_F_NO_INTERRUPT, Completion, Queue, USED_F_NO_NOTIFY};

#[cfg(test)]
mod tests;
