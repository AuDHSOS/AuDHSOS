// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The memory mapped configuration table: where the enhanced configuration
//! access mechanism puts the configuration space of a PCI segment group,
//! and which buses of that group the window covers.
//!
//! The layout is the *PCI Firmware Specification* 3.3, section 4.1.2: the
//! table header, eight reserved bytes, then one sixteen-byte allocation
//! structure per window. [`docs/pcisig/README.md`](../../../../docs/pcisig/README.md)
//! says why that document is not beside the code (D-124).
//!
//! Invariants: the walk over the allocations always ends, because every
//! one of them is the same fixed length; a table with more allocations
//! than this kernel holds is an error, not a truncation; an allocation
//! whose bus range runs backwards, or whose base is not page aligned, is
//! an error, because the window's length follows from the range and the
//! kernel maps the base as frames.

use kernel_types::PhysAddr;

use crate::error::AcpiError;
use crate::raw::{u8_at, u16_at, u64_at};
use crate::sdt::{SDT_HEADER_LEN, SdtHeader};

/// The four bytes that name this table.
pub const MCFG_SIGNATURE: [u8; 4] = *b"MCFG";

/// Number of bytes before the first allocation: the table header of
/// ACPI 6.6, section 5.2.6 and the eight bytes section 4.1.2 reserves.
pub const MCFG_HEADER_LEN: usize = SDT_HEADER_LEN + 8;

/// Number of bytes of one allocation structure.
pub const ALLOCATION_LEN: usize = 16;

/// Number of segment groups this kernel holds.
pub const MAX_ECAM_ALLOCATIONS: usize = 4;

/// Number of bytes the configuration space of one bus takes in the window,
/// which is thirty-two devices of eight functions of four kibibytes.
pub const BYTES_PER_BUS: u64 = 1 << 20;

/// The bits a base address has to have clear, which make it the page the
/// kernel maps it in.
const BASE_ALIGNMENT_MASK: u64 = 0xFFF;

/// One window: the configuration space of the buses of one segment group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ecam {
    /// Where the window starts.
    pub base: PhysAddr,
    /// The segment group the window covers.
    pub segment: u16,
    /// The first bus of the group the window holds.
    pub first_bus: u8,
    /// The last bus of the group the window holds.
    pub last_bus: u8,
}

impl Ecam {
    /// Number of buses the window covers, which is at least one because a
    /// range that runs backwards is refused at the parse.
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

    /// `true` when the window covers no bus, which no window this crate
    /// hands out does: a range that runs backwards is refused at the parse.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.last_bus < self.first_bus
    }
}

/// What the table says about the configuration windows of the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Mcfg {
    /// The windows, in the order the table names them.
    pub allocations: [Option<Ecam>; MAX_ECAM_ALLOCATIONS],
}

impl Mcfg {
    /// The first window the table named, which is the one the kernel
    /// reports: a machine of 3.1.1 has one segment group.
    #[must_use]
    pub fn first(&self) -> Option<Ecam> {
        self.allocations.iter().flatten().next().copied()
    }

    /// Number of windows the table named.
    #[must_use]
    pub fn count(&self) -> usize {
        self.allocations.iter().flatten().count()
    }
}

/// Reads the memory mapped configuration table out of `bytes`.
///
/// # Errors
///
/// The errors of [`SdtHeader::parse`]; [`AcpiError::Signature`] for a table
/// that is not an `MCFG` table; [`AcpiError::TooShort`] for a table that
/// ends inside an allocation; [`AcpiError::TooManyAllocations`] for a
/// machine with more segment groups than this kernel holds;
/// [`AcpiError::BusRange`] for an allocation whose last bus is below its
/// first; [`AcpiError::Unaligned`] for a base address that is no page;
/// [`AcpiError::Address`] for one that does not fit the physical address
/// width.
pub fn parse(bytes: &[u8]) -> Result<Mcfg, AcpiError> {
    let header = SdtHeader::parse(bytes)?.require(MCFG_SIGNATURE)?;
    let length = usize::try_from(header.length).unwrap_or(usize::MAX);
    let mut mcfg = Mcfg {
        allocations: [None; MAX_ECAM_ALLOCATIONS],
    };
    let mut offset = MCFG_HEADER_LEN;
    while offset < length {
        let allocation = read_allocation(bytes, offset, length)?;
        store(&mut mcfg.allocations, allocation).ok_or(AcpiError::TooManyAllocations)?;
        offset = offset.saturating_add(ALLOCATION_LEN);
    }
    Ok(mcfg)
}

/// The allocation at `offset`, checked against everything the kernel needs
/// of it.
fn read_allocation(bytes: &[u8], offset: usize, length: usize) -> Result<Ecam, AcpiError> {
    let end = offset.saturating_add(ALLOCATION_LEN);
    if end > length {
        return Err(AcpiError::TooShort(offset));
    }
    let raw = u64_at(bytes, offset).ok_or(AcpiError::TooShort(offset))?;
    let segment = u16_at(bytes, offset.saturating_add(8)).ok_or(AcpiError::TooShort(offset))?;
    let first_bus = u8_at(bytes, offset.saturating_add(10)).ok_or(AcpiError::TooShort(offset))?;
    let last_bus = u8_at(bytes, offset.saturating_add(11)).ok_or(AcpiError::TooShort(offset))?;
    if last_bus < first_bus {
        return Err(AcpiError::BusRange {
            first_bus,
            last_bus,
        });
    }
    if raw & BASE_ALIGNMENT_MASK != 0 {
        return Err(AcpiError::Unaligned(raw));
    }
    Ok(Ecam {
        base: PhysAddr::new(raw).map_err(|_| AcpiError::Address(raw))?,
        segment,
        first_bus,
        last_bus,
    })
}

/// Puts `value` into the first empty slot, or reports that there is none.
fn store<T>(slots: &mut [Option<T>], value: T) -> Option<()> {
    let slot = slots.iter_mut().find(|slot| slot.is_none())?;
    *slot = Some(value);
    Some(())
}
