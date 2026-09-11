// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Where a function's configuration space is: the address of a function,
//! the window one segment group's buses lie in, and the arithmetic of the
//! enhanced configuration access mechanism.
//!
//! The mechanism is the *PCI Express Base Specification* 6.0, section
//! 7.2.2: the offset of a function in the window is its bus, device, and
//! function number in bits 27:20, 19:15, and 14:12, and the register
//! offset in bits 11:0.
//!
//! Invariant: an [`Address`] exists only for a device and function a bus
//! has, and [`Window::offset_of`] answers only for an address the window
//! covers and an offset that lies whole inside a configuration space, so
//! the adapter that holds the mapping has no bound left to check.

use crate::error::PciError;

/// The highest device number a bus has.
pub const MAX_DEVICE: u8 = 31;

/// The highest function number a device has.
pub const MAX_FUNCTION: u8 = 7;

/// Number of bytes of one function's configuration space.
pub const CONFIG_SPACE_LEN: u16 = 4096;

/// Number of bytes of the header every function starts with.
pub const HEADER_LEN: u16 = 64;

/// The highest offset a whole word starts at.
const LAST_WORD: u16 = CONFIG_SPACE_LEN - 4;

/// Number of bytes one bus takes in the window.
pub const BYTES_PER_BUS: u64 = 1 << 20;

/// Number of bytes one device takes in the window.
pub const BYTES_PER_DEVICE: u64 = 1 << 15;

/// Number of bytes one function takes in the window.
pub const BYTES_PER_FUNCTION: u64 = 1 << 12;

/// Which function of which bus of which segment group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address {
    segment: u16,
    bus: u8,
    device: u8,
    function: u8,
}

impl Address {
    /// The address of one function.
    ///
    /// # Errors
    ///
    /// [`PciError::Device`] or [`PciError::Function`] for a number above
    /// what a bus and a device hold.
    pub const fn new(segment: u16, bus: u8, device: u8, function: u8) -> Result<Address, PciError> {
        if device > MAX_DEVICE {
            return Err(PciError::Device(device));
        }
        if function > MAX_FUNCTION {
            return Err(PciError::Function(function));
        }
        Ok(Address {
            segment,
            bus,
            device,
            function,
        })
    }

    /// The segment group.
    #[must_use]
    pub const fn segment(self) -> u16 {
        self.segment
    }

    /// The bus.
    #[must_use]
    pub const fn bus(self) -> u8 {
        self.bus
    }

    /// The device.
    #[must_use]
    pub const fn device(self) -> u8 {
        self.device
    }

    /// The function.
    #[must_use]
    pub const fn function(self) -> u8 {
        self.function
    }

    /// The same device's function zero, which is the one that says whether
    /// the device has any others.
    #[must_use]
    pub const fn first_function(self) -> Address {
        Address {
            function: 0,
            ..self
        }
    }

    /// The same device's function `function`, or `None` for a number above
    /// what a device holds.
    #[must_use]
    pub const fn with_function(self, function: u8) -> Option<Address> {
        if function > MAX_FUNCTION {
            return None;
        }
        Some(Address { function, ..self })
    }
}

/// The configuration space of every bus of one segment group, as the
/// memory mapped mechanism lays it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Window {
    segment: u16,
    first_bus: u8,
    last_bus: u8,
}

impl Window {
    /// The window over the buses `first_bus` to `last_bus` of `segment`.
    ///
    /// # Errors
    ///
    /// [`PciError::BusRange`] for a last bus below the first.
    pub const fn new(segment: u16, first_bus: u8, last_bus: u8) -> Result<Window, PciError> {
        if last_bus < first_bus {
            return Err(PciError::BusRange {
                first_bus,
                last_bus,
            });
        }
        Ok(Window {
            segment,
            first_bus,
            last_bus,
        })
    }

    /// The segment group the window covers.
    #[must_use]
    pub const fn segment(self) -> u16 {
        self.segment
    }

    /// The first bus the window covers.
    #[must_use]
    pub const fn first_bus(self) -> u8 {
        self.first_bus
    }

    /// The last bus the window covers.
    #[must_use]
    pub const fn last_bus(self) -> u8 {
        self.last_bus
    }

    /// Number of buses the window covers, which is at least one.
    #[must_use]
    pub fn buses(self) -> u16 {
        u16::from(self.last_bus)
            .saturating_sub(u16::from(self.first_bus))
            .saturating_add(1)
    }

    /// Number of bytes the window covers.
    #[must_use]
    pub fn len(self) -> u64 {
        u64::from(self.buses()).saturating_mul(BYTES_PER_BUS)
    }

    /// `true` when the window covers no bus at all, which no window built
    /// by [`Window::new`] does.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// `true` when `address` names a function the window holds.
    #[must_use]
    pub const fn holds(self, address: Address) -> bool {
        address.segment == self.segment
            && address.bus >= self.first_bus
            && address.bus <= self.last_bus
    }

    /// The byte offset of register `offset` of `address` inside the window.
    ///
    /// # Errors
    ///
    /// [`PciError::Segment`] or [`PciError::BusOutside`] for an address the
    /// window does not cover; [`PciError::Unaligned`] for an offset that is
    /// no whole word; [`PciError::Offset`] for a word that does not lie
    /// whole inside the configuration space.
    pub fn offset_of(self, address: Address, offset: u16) -> Result<u64, PciError> {
        if address.segment != self.segment {
            return Err(PciError::Segment {
                wanted: address.segment,
                window: self.segment,
            });
        }
        if !self.holds(address) {
            return Err(PciError::BusOutside {
                bus: address.bus,
                first_bus: self.first_bus,
                last_bus: self.last_bus,
            });
        }
        check_offset(offset)?;
        let bus = u64::from(address.bus.saturating_sub(self.first_bus));
        Ok(bus
            .saturating_mul(BYTES_PER_BUS)
            .saturating_add(u64::from(address.device).saturating_mul(BYTES_PER_DEVICE))
            .saturating_add(u64::from(address.function).saturating_mul(BYTES_PER_FUNCTION))
            .saturating_add(u64::from(offset)))
    }

    /// Every address the window covers, bus by bus, device by device,
    /// function by function.
    pub fn addresses(self) -> impl Iterator<Item = Address> {
        let segment = self.segment;
        (self.first_bus..=self.last_bus).flat_map(move |bus| {
            (0..=MAX_DEVICE).flat_map(move |device| {
                (0..=MAX_FUNCTION)
                    .filter_map(move |function| Address::new(segment, bus, device, function).ok())
            })
        })
    }
}

/// Checks that a word at `offset` lies whole inside a configuration space.
///
/// # Errors
///
/// [`PciError::Unaligned`] for an offset that is no whole word;
/// [`PciError::Offset`] for one whose word leaves the space.
pub const fn check_offset(offset: u16) -> Result<(), PciError> {
    if offset & 0b11 != 0 {
        return Err(PciError::Unaligned(offset));
    }
    if offset > LAST_WORD {
        return Err(PciError::Offset(offset));
    }
    Ok(())
}
