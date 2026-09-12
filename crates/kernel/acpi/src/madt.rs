// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The multiple APIC description table: where the local APIC is, where the
//! I/O APICs are, and which of the ISA interrupt lines the firmware wired
//! somewhere other than the default.
//!
//! The layout is the *ACPI Specification* 6.6, section 5.2.12: the table
//! header, the local APIC address, the flags, then the interrupt
//! controller structures. Sections 5.2.12.2, 5.2.12.3, 5.2.12.5 and
//! 5.2.12.8 give the four structures this parser reads.
//!
//! Invariants: the walk over the entries always ends, because an entry of
//! length zero is an error and every other entry advances the offset by at
//! least two; an entry that leaves the table is an error, not a truncation;
//! a table with more I/O APICs or more overrides than this kernel holds is
//! an error, not a truncation.

use kernel_types::PhysAddr;

use crate::error::AcpiError;
use crate::raw::{array_at, u8_at, u16_at, u32_at, u64_at};
use crate::sdt::{SDT_HEADER_LEN, SdtHeader};

/// The four bytes that name this table.
pub const MADT_SIGNATURE: [u8; 4] = *b"APIC";

/// Number of bytes before the first entry: the table header, the local
/// APIC address, and the flags, from section 5.2.12.
pub const MADT_HEADER_LEN: usize = SDT_HEADER_LEN + 8;

/// Number of I/O APICs this kernel holds.
pub const MAX_IO_APICS: usize = 4;

/// Number of interrupt source overrides this kernel holds.
pub const MAX_OVERRIDES: usize = 16;

/// The flag that says the machine also has the two legacy interrupt
/// controllers, which the kernel then has to mask. Section 5.2.12 defines
/// it as bit zero and the only flag of the table.
pub const PCAT_COMPAT: u32 = 1;

/// Entry type: a processor's local APIC, section 5.2.12.2.
const ENTRY_LOCAL_APIC: u8 = 0;

/// Entry type: an I/O APIC, section 5.2.12.3.
const ENTRY_IO_APIC: u8 = 1;

/// Entry type: an interrupt source override, section 5.2.12.5.
const ENTRY_OVERRIDE: u8 = 2;

/// Entry type: a 64-bit address of the local APIC that replaces the 32-bit
/// one in the table header, section 5.2.12.8.
const ENTRY_LOCAL_APIC_OVERRIDE: u8 = 5;

/// Length of a processor local APIC entry, section 5.2.12.2.
const LOCAL_APIC_LEN: u8 = 8;

/// Length of an I/O APIC entry, section 5.2.12.3.
const IO_APIC_LEN: u8 = 12;

/// Length of an interrupt source override entry, section 5.2.12.5.
const OVERRIDE_LEN: u8 = 10;

/// Length of a local APIC address override entry, section 5.2.12.8.
const LOCAL_APIC_OVERRIDE_LEN: u8 = 12;

/// The bus the ISA interrupt lines are on, the only one section 5.2.12.5
/// overrides.
pub const ISA_BUS: u8 = 0;

/// One I/O APIC of the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IoApic {
    /// The identifier the firmware gave it.
    pub id: u8,
    /// The address of its register window.
    pub address: PhysAddr,
    /// The first global system interrupt it serves.
    pub gsi_base: u32,
}

/// How an interrupt line is asserted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Polarity {
    /// The line is asserted high, which is what an ISA line does unless
    /// the firmware says otherwise.
    #[default]
    ActiveHigh,
    /// The line is asserted low.
    ActiveLow,
}

/// When an interrupt line is taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// On the edge, which is what an ISA line does unless the firmware
    /// says otherwise.
    #[default]
    Edge,
    /// While the line is held, which needs an end-of-interrupt before the
    /// next one arrives.
    Level,
}

/// One line the firmware wired somewhere other than the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Override {
    /// The bus the source line is on; only [`ISA_BUS`] exists.
    pub bus: u8,
    /// The line as the ISA numbering knows it.
    pub source: u8,
    /// The global system interrupt it actually reaches.
    pub gsi: u32,
    /// Polarity in bits 0 and 1, trigger mode in bits 2 and 3.
    pub flags: u16,
}

impl Override {
    /// The polarity the flags name; the ISA default where they say
    /// `conforms` or a value section 5.2.12.5 reserves.
    #[must_use]
    pub const fn polarity(self) -> Polarity {
        if self.flags & 0b11 == 0b11 {
            Polarity::ActiveLow
        } else {
            Polarity::ActiveHigh
        }
    }

    /// The trigger mode the flags name; the ISA default where they say
    /// `conforms` or a value section 5.2.12.5 reserves.
    #[must_use]
    pub const fn trigger(self) -> Trigger {
        if (self.flags >> 2) & 0b11 == 0b11 {
            Trigger::Level
        } else {
            Trigger::Edge
        }
    }
}

/// Where a line goes and how it is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Routing {
    /// The global system interrupt the line reaches.
    pub gsi: u32,
    /// How the line is asserted.
    pub polarity: Polarity,
    /// When the line is taken.
    pub trigger: Trigger,
}

/// What the table says about the interrupt hardware of the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Madt {
    /// The address of the local APIC register window, after a local APIC
    /// address override if the table carries one.
    pub lapic_address: PhysAddr,
    /// The flags of the table; [`PCAT_COMPAT`] is the only one defined.
    pub flags: u32,
    /// Number of processor local APIC entries, which this release counts
    /// and does nothing else with.
    pub processors: usize,
    /// The I/O APICs, in the order the table names them.
    pub io_apics: [Option<IoApic>; MAX_IO_APICS],
    /// The interrupt source overrides, in the order the table names them.
    pub overrides: [Option<Override>; MAX_OVERRIDES],
}

impl Madt {
    /// `true` if the machine also has the two legacy interrupt
    /// controllers.
    #[must_use]
    pub const fn has_legacy_pic(&self) -> bool {
        self.flags & PCAT_COMPAT != 0
    }

    /// Where ISA line `irq` goes and how it is taken. Without an override
    /// the line is the global system interrupt of the same number, taken
    /// on the edge and asserted high.
    #[must_use]
    pub fn route_isa(&self, irq: u8) -> Routing {
        match self.override_for(irq) {
            Some(entry) => Routing {
                gsi: entry.gsi,
                polarity: entry.polarity(),
                trigger: entry.trigger(),
            },
            None => Routing {
                gsi: u32::from(irq),
                polarity: Polarity::ActiveHigh,
                trigger: Trigger::Edge,
            },
        }
    }

    /// The override of ISA line `irq`, if the table names one.
    #[must_use]
    pub fn override_for(&self, irq: u8) -> Option<Override> {
        self.overrides
            .iter()
            .flatten()
            .find(|entry| entry.bus == ISA_BUS && entry.source == irq)
            .copied()
    }

    /// The I/O APIC that serves `gsi`: the one with the highest base at or
    /// below it, because the table says where each one starts and only the
    /// hardware says how far it reaches.
    #[must_use]
    pub fn io_apic_for(&self, gsi: u32) -> Option<IoApic> {
        self.io_apics
            .iter()
            .flatten()
            .filter(|apic| apic.gsi_base <= gsi)
            .max_by_key(|apic| apic.gsi_base)
            .copied()
    }

    /// Number of I/O APICs the table named.
    #[must_use]
    pub fn io_apic_count(&self) -> usize {
        self.io_apics.iter().flatten().count()
    }

    /// Number of interrupt source overrides the table named.
    #[must_use]
    pub fn override_count(&self) -> usize {
        self.overrides.iter().flatten().count()
    }
}

/// Reads the multiple APIC description table out of `bytes`.
///
/// # Errors
///
/// The errors of [`SdtHeader::parse`]; [`AcpiError::Signature`] for a table
/// that is not an `APIC` table; [`AcpiError::EntryLengthZero`],
/// [`AcpiError::EntryTruncated`], or [`AcpiError::EntryLength`] for an
/// entry the walk cannot use; [`AcpiError::TooManyIoApics`] or
/// [`AcpiError::TooManyOverrides`] for a machine larger than this kernel
/// holds; [`AcpiError::Address`] for an address that does not fit the
/// physical address width.
pub fn parse(bytes: &[u8]) -> Result<Madt, AcpiError> {
    let header = SdtHeader::parse(bytes)?.require(MADT_SIGNATURE)?;
    let length = usize::try_from(header.length).unwrap_or(usize::MAX);
    let lapic = u32_at(bytes, SDT_HEADER_LEN).ok_or(AcpiError::TooShort(bytes.len()))?;
    let flags =
        u32_at(bytes, SDT_HEADER_LEN.saturating_add(4)).ok_or(AcpiError::TooShort(bytes.len()))?;
    let mut madt = Madt {
        lapic_address: address(u64::from(lapic))?,
        flags,
        processors: 0,
        io_apics: [None; MAX_IO_APICS],
        overrides: [None; MAX_OVERRIDES],
    };
    let mut offset = MADT_HEADER_LEN;
    while offset < length {
        offset = read_entry(bytes, offset, length, &mut madt)?;
    }
    Ok(madt)
}

/// Reads the entry at `offset` into `madt` and returns the offset of the
/// next one.
fn read_entry(
    bytes: &[u8],
    offset: usize,
    length: usize,
    madt: &mut Madt,
) -> Result<usize, AcpiError> {
    let [kind, announced] = array_at::<2>(bytes, offset).ok_or(AcpiError::TooShort(offset))?;
    if announced == 0 {
        return Err(AcpiError::EntryLengthZero(offset));
    }
    let end = offset.saturating_add(usize::from(announced));
    if end > length {
        return Err(AcpiError::EntryTruncated {
            kind,
            length: announced,
        });
    }
    match kind {
        ENTRY_LOCAL_APIC => {
            expect_length(kind, announced, LOCAL_APIC_LEN)?;
            madt.processors = madt.processors.saturating_add(1);
        }
        ENTRY_IO_APIC => {
            expect_length(kind, announced, IO_APIC_LEN)?;
            let apic = read_io_apic(bytes, offset)?;
            store(&mut madt.io_apics, apic).ok_or(AcpiError::TooManyIoApics)?;
        }
        ENTRY_OVERRIDE => {
            expect_length(kind, announced, OVERRIDE_LEN)?;
            let entry = read_override(bytes, offset)?;
            store(&mut madt.overrides, entry).ok_or(AcpiError::TooManyOverrides)?;
        }
        ENTRY_LOCAL_APIC_OVERRIDE => {
            expect_length(kind, announced, LOCAL_APIC_OVERRIDE_LEN)?;
            let raw =
                u64_at(bytes, offset.saturating_add(4)).ok_or(AcpiError::TooShort(bytes.len()))?;
            madt.lapic_address = address(raw)?;
        }
        _ => {}
    }
    Ok(end)
}

/// Checks that an entry of a type this parser reads has the length its
/// section of ACPI 6.6 gives it.
const fn expect_length(kind: u8, announced: u8, wanted: u8) -> Result<(), AcpiError> {
    if announced == wanted {
        Ok(())
    } else {
        Err(AcpiError::EntryLength {
            kind,
            length: announced,
        })
    }
}

/// The I/O APIC the entry at `offset` describes.
fn read_io_apic(bytes: &[u8], offset: usize) -> Result<IoApic, AcpiError> {
    let id = u8_at(bytes, offset.saturating_add(2)).ok_or(AcpiError::TooShort(bytes.len()))?;
    let raw = u32_at(bytes, offset.saturating_add(4)).ok_or(AcpiError::TooShort(bytes.len()))?;
    let gsi_base =
        u32_at(bytes, offset.saturating_add(8)).ok_or(AcpiError::TooShort(bytes.len()))?;
    Ok(IoApic {
        id,
        address: address(u64::from(raw))?,
        gsi_base,
    })
}

/// The override the entry at `offset` describes.
fn read_override(bytes: &[u8], offset: usize) -> Result<Override, AcpiError> {
    let bus = u8_at(bytes, offset.saturating_add(2)).ok_or(AcpiError::TooShort(bytes.len()))?;
    let source = u8_at(bytes, offset.saturating_add(3)).ok_or(AcpiError::TooShort(bytes.len()))?;
    let gsi = u32_at(bytes, offset.saturating_add(4)).ok_or(AcpiError::TooShort(bytes.len()))?;
    let flags = u16_at(bytes, offset.saturating_add(8)).ok_or(AcpiError::TooShort(bytes.len()))?;
    Ok(Override {
        bus,
        source,
        gsi,
        flags,
    })
}

/// Puts `value` into the first empty slot, or reports that there is none.
fn store<T>(slots: &mut [Option<T>], value: T) -> Option<()> {
    let slot = slots.iter_mut().find(|slot| slot.is_none())?;
    *slot = Some(value);
    Some(())
}

/// The address a field names, if it is one the machine can have.
fn address(raw: u64) -> Result<PhysAddr, AcpiError> {
    PhysAddr::new(raw).map_err(|_| AcpiError::Address(raw))
}
