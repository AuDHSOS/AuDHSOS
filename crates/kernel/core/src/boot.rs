// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The boot sequence: what the kernel reports before it has anything to
//! schedule.
//!
//! Invariant: the sequence touches the machine only through the traits it
//! is given, so that it runs unchanged on the host and in QEMU.

use audhsos_abi::layout::PHYS_WINDOW_BASE;
use audhsos_abi::{Ecam, Framebuffer, FramebufferFormat};
use kernel_hal_api::console::DebugConsole;
use kernel_hal_api::exit::{ExitStatus, TestExit};
use kernel_hal_api::platform::{MemoryRegion, MemoryRegionKind, Platform};

use crate::memory::MemoryError;
use crate::println;
use crate::state::{KERNEL, KernelState};

/// The name the kernel reports.
pub const NAME: &str = "AuDHSOS";

/// The version the kernel reports.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Why the kernel cannot go on after reading what the loader left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BootError {
    /// The loader mapped the physical memory window somewhere else than
    /// the address space layout says.
    WindowBase(u64),
    /// The loader reported no usable memory.
    NoUsableMemory,
    /// The kernel could not take its memory over.
    Memory(MemoryError),
}

impl core::fmt::Display for BootError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BootError::WindowBase(base) => write!(
                f,
                "the physical window is at {base:#x}, not at {PHYS_WINDOW_BASE:#x}"
            ),
            BootError::NoUsableMemory => f.write_str("the loader reported no usable memory"),
            BootError::Memory(error) => write!(f, "{error}"),
        }
    }
}

/// Prints the banner, the physical memory window, and every memory region
/// the loader reported, then initializes the kernel state.
///
/// The caller halts afterwards; this function returns so that the same
/// sequence can run in a host test.
///
/// # Errors
///
/// [`BootError`] if the machine is not one the kernel can run on.
pub fn run(platform: &impl Platform, console: &mut impl DebugConsole) -> Result<(), BootError> {
    println!(console, "{NAME} {VERSION}");
    println!(
        console,
        "physical window at {}",
        platform.physical_window_base()
    );
    match platform.acpi_rsdp() {
        Some(rsdp) => println!(console, "acpi root pointer at {rsdp}"),
        None => println!(console, "no acpi root pointer"),
    }
    let regions = platform.memory_regions();
    println!(console, "{} memory regions", regions.len());
    for region in regions {
        print_region(console, region);
    }
    println!(console, "usable {} KiB", usable_kib(regions));
    print_framebuffer(console, platform.framebuffer());
    print_ecam(console, platform.ecam());
    let window = platform.physical_window_base().as_u64();
    if window != PHYS_WINDOW_BASE {
        return Err(BootError::WindowBase(window));
    }
    if usable_kib(regions) == 0 {
        return Err(BootError::NoUsableMemory);
    }
    let _ = KERNEL.init(KernelState::new());
    Ok(())
}

/// Reports the framebuffer the loader described, or that there is none.
///
/// The line carries the `[info]` prefix of
/// [03 3.1.7](../../../../docs/03-target-platform.md#317-test-exit-protocol),
/// because the runner reads it: the image that boots without a graphics
/// adapter is a test of exactly this line.
fn print_framebuffer(console: &mut impl DebugConsole, framebuffer: Option<Framebuffer>) {
    match framebuffer {
        Some(screen) => println!(
            console,
            "[info] framebuffer={}x{} stride={} format={} at {:#x}",
            screen.width,
            screen.height,
            screen.stride,
            format_name(screen.format),
            screen.phys_start
        ),
        None => println!(console, "[info] framebuffer=absent"),
    }
}

/// Reports the configuration window of the bus, or that the firmware
/// published none.
///
/// The line carries the `[info]` prefix of
/// [03 3.1.7](../../../../docs/03-target-platform.md#317-test-exit-protocol)
/// for the same reason the framebuffer line does: the runner reads it, and
/// a machine started without the network device is a run of exactly this
/// line.
fn print_ecam(console: &mut impl DebugConsole, ecam: Option<Ecam>) {
    match ecam {
        Some(window) => println!(
            console,
            "[info] ecam={:#x} segment={} buses={}..={}",
            window.base,
            window.segment,
            window.first_bus,
            window.last_bus
        ),
        None => println!(console, "[info] ecam=absent"),
    }
}

/// The name of a pixel format, as the line above spells it.
const fn format_name(format: FramebufferFormat) -> &'static str {
    match format {
        FramebufferFormat::Rgbx8888 => "rgbx8888",
        FramebufferFormat::Bgrx8888 => "bgrx8888",
    }
}

/// Reports one region as `start-end kind`.
fn print_region(console: &mut impl DebugConsole, region: &MemoryRegion) {
    let end = region.start.checked_add(region.len);
    match end {
        Some(end) => println!(
            console,
            "  {}-{} {}",
            region.start,
            end,
            kind_name(region.kind)
        ),
        None => println!(
            console,
            "  {}+{} {}",
            region.start,
            region.len,
            kind_name(region.kind)
        ),
    }
}

/// The name of a region kind, as the report writes it.
#[must_use]
pub const fn kind_name(kind: MemoryRegionKind) -> &'static str {
    match kind {
        MemoryRegionKind::Usable => "usable",
        MemoryRegionKind::Reserved => "reserved",
        MemoryRegionKind::AcpiReclaimable => "acpi-reclaimable",
        MemoryRegionKind::AcpiNvs => "acpi-nvs",
        MemoryRegionKind::MmioReserved => "mmio",
        MemoryRegionKind::Kernel => "kernel",
        MemoryRegionKind::BootImage => "boot-image",
        MemoryRegionKind::PageTables => "page-tables",
        MemoryRegionKind::BootStack => "boot-stack",
        MemoryRegionKind::BootInfo => "boot-info",
    }
}

/// The usable memory in kibibytes.
#[must_use]
pub fn usable_kib(regions: &[MemoryRegion]) -> u64 {
    regions
        .iter()
        .filter(|region| region.kind == MemoryRegionKind::Usable)
        .fold(0u64, |total, region| total.saturating_add(region.len >> 10))
}

/// The usable memory in bytes.
#[must_use]
pub fn usable_bytes(regions: &[MemoryRegion]) -> u64 {
    regions
        .iter()
        .filter(|region| region.kind == MemoryRegionKind::Usable)
        .fold(0u64, |total, region| total.saturating_add(region.len))
}

/// Reports that the kernel has nothing left to do and ends the machine
/// with a success.
pub fn finish(console: &mut impl DebugConsole, exit: &mut impl TestExit) {
    println!(console, "boot complete");
    exit.exit(ExitStatus::Success);
}

/// Reports why the kernel cannot go on and ends the machine with a
/// failure.
pub fn abort(error: BootError, console: &mut impl DebugConsole, exit: &mut impl TestExit) {
    println!(console, "[boot] {error}");
    exit.exit(ExitStatus::Failure);
}

impl From<MemoryError> for BootError {
    fn from(error: MemoryError) -> Self {
        BootError::Memory(error)
    }
}
