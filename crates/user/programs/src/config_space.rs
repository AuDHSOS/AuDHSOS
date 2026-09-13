// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The configuration space of one bus, over the mapping of that bus.
//!
//! A program that enumerates the bus maps one bus of the configuration
//! window at a time and reads the four-byte words of every function through
//! that mapping. This is the whole of the adapter: the offset arithmetic is
//! [`Window::offset_of`], the access is [`Mmio`], and nothing here knows
//! either layout.
//!
//! Two programs use it: `app-lspci`, which reports what the bus holds, and
//! the root task, which looks for the virtio block device.

use pci::address::{Address, Window};
use pci::space::ConfigSpace;
use user_sys_x86_64::Mmio;

/// One bus of the configuration window, as `pci` reads it.
pub struct MappedSpace<'a> {
    window: Window,
    mmio: Mmio<'a>,
}

impl<'a> MappedSpace<'a> {
    /// The space `window` describes, over the bytes `mmio` covers.
    #[must_use]
    pub const fn new(window: Window, mmio: Mmio<'a>) -> Self {
        MappedSpace { window, mmio }
    }
}

impl ConfigSpace for MappedSpace<'_> {
    fn read_u32(&self, address: Address, offset: u16) -> Option<u32> {
        let at = self.window.offset_of(address, offset).ok()?;
        self.mmio.read_u32(usize::try_from(at).ok()?)
    }

    fn write_u32(&mut self, address: Address, offset: u16, value: u32) {
        let Ok(at) = self.window.offset_of(address, offset) else {
            return;
        };
        let Ok(at) = usize::try_from(at) else {
            return;
        };
        let _written = self.mmio.write_u32(at, value);
    }
}
