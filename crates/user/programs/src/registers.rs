// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The registers of a virtio device, over the window the root task handed
//! the driver.
//!
//! `driver-virtio-blk` names a structure and an offset inside it and never
//! computes an address. This turns that pair into an offset into the
//! window: the structure's own offset plus the given one, checked against
//! the structure's length, and reached through [`Mmio`], which checks it a
//! second time against the length of the mapping.
//!
//! The four places come from the startup message, and their offsets count
//! from the start of the window (see `Role::BlockCommon`), so nothing here
//! knows where the base address register began.
//!
//! Invariant: an access that leaves the structure it names reads zero and
//! writes nothing. Neither trait method carries a refusal — a read answers
//! `u64` and a write answers nothing — so a substitute is what there is,
//! and the check above it is what keeps the substitute unreachable.

use audhsos_abi::startup::Location;
use driver_virtio_blk::registers::{Registers, Structure, Width};
use user_sys_x86_64::Mmio;

/// Where the window of the virtio block device is mapped in the driver.
///
/// It lies where the other mappings of these programs lie, far above where
/// any program is linked and far below the IPC buffers, and clear of
/// [`crate::mapping::SCRATCH`] so that a driver can hold both at once.
pub const BLOCK_WINDOW: u64 = 0x0000_4400_0000_0000;

/// The registers of one device, over the bytes of its window.
pub struct Window<'a> {
    mmio: Mmio<'a>,
    places: [Location; 4],
}

impl<'a> Window<'a> {
    /// The registers `places` describe, over `bytes`.
    ///
    /// The order of `places` is the order of [`Structure`]: common,
    /// notify, interrupt status, device configuration.
    #[must_use]
    pub const fn new(bytes: &'a mut [u8], places: [Location; 4]) -> Self {
        Window {
            mmio: Mmio::of(bytes),
            places,
        }
    }

    /// Where `structure` begins in the window and how long it is.
    const fn place(&self, structure: Structure) -> Location {
        let [common, notify, isr, device] = self.places;
        match structure {
            Structure::Common => common,
            Structure::Notify => notify,
            Structure::Isr => isr,
            Structure::Device => device,
        }
    }

    /// The offset in the window `structure` and `offset` name, or `None`
    /// for an access that would leave that structure.
    fn at(&self, structure: Structure, offset: u16, width: Width) -> Option<usize> {
        let place = self.place(structure);
        let end = u32::from(offset).checked_add(u32::from(width.bytes()))?;
        if end > place.len {
            return None;
        }
        usize::try_from(place.offset.checked_add(u32::from(offset))?).ok()
    }
}

impl Registers for Window<'_> {
    fn read(&self, structure: Structure, offset: u16, width: Width) -> u64 {
        let Some(at) = self.at(structure, offset, width) else {
            return 0;
        };
        let read = match width {
            Width::B8 => self.mmio.read_u8(at).map(u64::from),
            Width::B16 => self.mmio.read_u16(at).map(u64::from),
            Width::B32 => self.mmio.read_u32(at).map(u64::from),
            Width::B64 => self.mmio.read_u64(at),
        };
        read.unwrap_or(0)
    }

    fn write(&mut self, structure: Structure, offset: u16, width: Width, value: u64) {
        let Some(at) = self.at(structure, offset, width) else {
            return;
        };
        let _written = match width {
            Width::B8 => self.mmio.write_u8(at, low_u8(value)),
            Width::B16 => self.mmio.write_u16(at, low_u16(value)),
            Width::B32 => self.mmio.write_u32(at, low_u32(value)),
            Width::B64 => self.mmio.write_u64(at, value),
        };
    }
}

/// The low byte of a register value.
#[expect(
    clippy::as_conversions,
    reason = "the mask is the truncation, so the cast drops nothing"
)]
const fn low_u8(value: u64) -> u8 {
    (value & 0xFF) as u8
}

/// The low two bytes of a register value.
#[expect(
    clippy::as_conversions,
    reason = "the mask is the truncation, so the cast drops nothing"
)]
const fn low_u16(value: u64) -> u16 {
    (value & 0xFFFF) as u16
}

/// The low four bytes of a register value.
#[expect(
    clippy::as_conversions,
    reason = "the mask is the truncation, so the cast drops nothing"
)]
const fn low_u32(value: u64) -> u32 {
    (value & 0xFFFF_FFFF) as u32
}
