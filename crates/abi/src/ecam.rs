// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The window the configuration space of a PCI segment group lies in.
//!
//! It is an interface type and not a boot one: the loader does not find it,
//! the kernel reads it out of the `MCFG` table of the firmware, and
//! `system_info` reports it to the root task, which makes the device memory
//! object of the program that enumerates out of it.

/// Number of bytes the configuration space of one bus takes in the window,
/// which is thirty-two devices of eight functions of four kibibytes.
pub const BYTES_PER_BUS: u64 = 1 << 20;

/// Where the configuration space of one segment group is, and which of its
/// buses the window covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ecam {
    /// Physical base of the window, frame-aligned.
    pub base: u64,
    /// The segment group the window covers.
    pub segment: u16,
    /// The first bus of the group the window holds.
    pub first_bus: u8,
    /// The last bus of the group the window holds.
    pub last_bus: u8,
}

impl Ecam {
    /// Number of buses the window covers, or zero for an empty window.
    #[must_use]
    pub fn buses(self) -> u16 {
        if self.is_empty() {
            return 0;
        }
        u16::from(self.last_bus)
            .saturating_sub(u16::from(self.first_bus))
            .saturating_add(1)
    }

    /// Number of bytes the window covers.
    #[must_use]
    pub fn len(self) -> u64 {
        u64::from(self.buses()).saturating_mul(BYTES_PER_BUS)
    }

    /// `true` for an absent base or a reversed bus range.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.base == 0 || self.last_bus < self.first_bus
    }
}
