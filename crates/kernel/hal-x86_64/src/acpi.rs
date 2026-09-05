// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Finding the ACPI tables of the machine and handing their bytes to the
//! parsers in `kernel-acpi`.
//!
//! Invariants: no range is read before it has been checked against the
//! memory the firmware reported, so a root pointer that names nothing
//! makes the kernel report and not fault; the bytes leave this module as
//! a slice and every decision about them is made by a safe parser.

use kernel_acpi::error::AcpiError;
use kernel_acpi::madt::{MADT_SIGNATURE, Madt};
use kernel_acpi::rsdp::{RSDP_LEN, parse_rsdp};
use kernel_acpi::sdt::{RootTable, SDT_HEADER_LEN, SdtHeader, announced_length};
use kernel_hal_api::platform::{MemoryRegionKind, Platform};
use kernel_types::PhysAddr;

use crate::bootinfo::X86Platform;
use crate::window::PhysicalWindow;

/// Why the kernel could not read the tables of the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableError {
    /// The loader found no root pointer, so the machine has no ACPI or the
    /// firmware did not report it.
    NoRootPointer,
    /// A table lies outside the memory the window maps, so the firmware
    /// named something the kernel cannot reach.
    Unreachable(PhysAddr),
    /// A table was rejected by its parser.
    Table(AcpiError),
    /// The root table names no multiple APIC description table.
    NoMadt,
}

impl core::fmt::Display for TableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TableError::NoRootPointer => f.write_str("the loader reported no acpi root pointer"),
            TableError::Unreachable(address) => {
                write!(
                    f,
                    "the table at {address} is outside the memory the window maps"
                )
            }
            TableError::Table(error) => write!(f, "{error}"),
            TableError::NoMadt => f.write_str("no table of the machine is an apic table"),
        }
    }
}

impl From<AcpiError> for TableError {
    fn from(error: AcpiError) -> Self {
        TableError::Table(error)
    }
}

/// Reads the multiple APIC description table of the machine.
///
/// The walk is the one the specification prescribes: the root pointer, the
/// root table it names, then every table the root table names until one
/// carries the `APIC` signature.
///
/// # Errors
///
/// [`TableError`] for a machine whose tables the kernel cannot read.
///
/// # Safety
///
/// The tables the loader built must be active, so that the window maps
/// every frame of the memory the firmware reported.
pub unsafe fn find_madt(platform: &X86Platform) -> Result<Madt, TableError> {
    let limit = window_limit(platform);
    // SAFETY: the caller promises that the loader's tables are active, so
    // the window maps every frame of memory; every range this value hands
    // out is checked against `limit` first.
    let window = unsafe { PhysicalWindow::kernel() };
    let address = platform.acpi_rsdp().ok_or(TableError::NoRootPointer)?;
    let pointer: [u8; RSDP_LEN] = read_array(window, limit, address)?;
    let rsdp = parse_rsdp(&pointer)?;
    let root_bytes = read_table(&window, limit, rsdp.root())?;
    let root = RootTable::parse(root_bytes)?;
    for entry in root.addresses() {
        let bytes = read_table(&window, limit, entry)?;
        let header = SdtHeader::parse(bytes)?;
        if header.signature == MADT_SIGNATURE {
            return Ok(kernel_acpi::madt::parse(bytes)?);
        }
    }
    Err(TableError::NoMadt)
}

/// The first byte above the memory the window maps, which is what the
/// loader sized the window over.
fn window_limit(platform: &impl Platform) -> u64 {
    platform
        .memory_regions()
        .iter()
        .filter(|region| is_window_memory(region.kind))
        .filter_map(|region| region.start.checked_add(region.len))
        .fold(0u64, |highest, end| highest.max(end.as_u64()))
}

/// `true` if the window maps a region of this kind. The loader sizes the
/// window over the same set.
const fn is_window_memory(kind: MemoryRegionKind) -> bool {
    matches!(
        kind,
        MemoryRegionKind::Usable | MemoryRegionKind::AcpiReclaimable | MemoryRegionKind::AcpiNvs
    )
}

/// The `len` bytes at `start`, if the whole range is inside the memory the
/// window maps.
fn read_bytes(
    window: &PhysicalWindow,
    limit: u64,
    start: PhysAddr,
    len: usize,
) -> Result<&[u8], TableError> {
    let end = u64::try_from(len)
        .ok()
        .and_then(|len| start.as_u64().checked_add(len))
        .ok_or(TableError::Unreachable(start))?;
    if end > limit {
        return Err(TableError::Unreachable(start));
    }
    // SAFETY: the range was just checked to lie inside the memory the
    // window maps, which the caller of `find_madt` promises is mapped.
    unsafe { window.bytes(start, len) }.ok_or(TableError::Unreachable(start))
}

/// The `N` bytes at `start`.
fn read_array<const N: usize>(
    window: PhysicalWindow,
    limit: u64,
    start: PhysAddr,
) -> Result<[u8; N], TableError> {
    let bytes = read_bytes(&window, limit, start, N)?;
    <[u8; N]>::try_from(bytes).map_err(|_| TableError::Unreachable(start))
}

/// The whole table at `start`: its header says how long it is, so the read
/// happens twice.
fn read_table(window: &PhysicalWindow, limit: u64, start: PhysAddr) -> Result<&[u8], TableError> {
    let head: [u8; SDT_HEADER_LEN] = read_array(*window, limit, start)?;
    let length = usize::try_from(announced_length(&head)).unwrap_or(usize::MAX);
    read_bytes(window, limit, start, length)
}
