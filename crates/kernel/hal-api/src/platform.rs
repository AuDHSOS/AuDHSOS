// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The boot platform: what the loader left behind.

use audhsos_abi::{Ecam, Framebuffer};
use kernel_types::{PhysAddr, VirtAddr};

/// What a physical memory region holds at boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemoryRegionKind {
    /// Free RAM.
    Usable,
    /// Not usable by the kernel.
    Reserved,
    /// ACPI tables; usable after they have been read.
    AcpiReclaimable,
    /// ACPI non-volatile storage; never usable.
    AcpiNvs,
    /// Memory-mapped device registers.
    MmioReserved,
    /// The kernel image.
    Kernel,
    /// The boot image.
    BootImage,
    /// The initial page tables built by the loader.
    PageTables,
    /// The boot stack.
    BootStack,
    /// The boot information structure.
    BootInfo,
}

/// A physical memory region as reported at boot. Regions are byte-granular
/// and may overlap; the kernel normalizes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryRegion {
    /// First byte of the region.
    pub start: PhysAddr,
    /// Length in bytes.
    pub len: u64,
    /// What the region holds.
    pub kind: MemoryRegionKind,
}

/// The boot platform.
pub trait Platform {
    /// Every memory region the loader reported.
    fn memory_regions(&self) -> &[MemoryRegion];

    /// Virtual base of the physical memory window.
    fn physical_window_base(&self) -> VirtAddr;

    /// The framebuffer the loader found, if the machine has one. It is
    /// what `system_info` reports and what the root task builds the device
    /// memory object of the display server from.
    fn framebuffer(&self) -> Option<Framebuffer>;

    /// Physical address of the ACPI root pointer, if the firmware provided
    /// one.
    fn acpi_rsdp(&self) -> Option<PhysAddr>;

    /// The configuration window of the bus, if the firmware published an
    /// `MCFG` table naming one. It is what `system_info` reports and what
    /// the root task builds the device memory object of the program that
    /// enumerates from.
    fn ecam(&self) -> Option<Ecam>;
}
