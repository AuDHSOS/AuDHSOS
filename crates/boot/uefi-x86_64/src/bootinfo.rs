// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Filling the page the kernel reads the machine out of.
//!
//! Invariants: the regions come from the memory map the loader read
//! immediately before it left the boot services; a framebuffer is only
//! reported when the region array can describe it the way the kernel
//! demands; a root pointer that lies outside every region is dropped
//! instead of being reported.

use core::fmt;

use audhsos_abi::boot_info::{
    BOOT_INFO_PAGE_LEN, BootInfoHeader, BootInfoWriter, BootRegion, BootRegionKind, Framebuffer,
    WallClockSource,
};
use audhsos_abi::layout::{MAX_BOOT_REGIONS, PHYS_WINDOW_BASE};
use audhsos_uefi::memory_map::{ConversionError, descriptors, to_boot_regions};

/// A physical range, as the boot information reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Range {
    /// First byte.
    pub(crate) start: u64,
    /// Length in bytes.
    pub(crate) len: u64,
}

/// The four fixed ranges and the root pointer.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Ranges {
    /// The kernel image.
    pub(crate) kernel: Range,
    /// The boot image.
    pub(crate) boot_image: Range,
    /// The initial page tables.
    pub(crate) page_tables: Range,
    /// The boot stack, guard page excluded.
    pub(crate) boot_stack: Range,
    /// The ACPI root pointer, `0` if the firmware reported none.
    pub(crate) rsdp: u64,
}

/// Why the boot information could not be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteError {
    /// The memory map could not be turned into regions.
    Conversion(ConversionError),
    /// The structure does not fit into the page.
    TooLarge,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::Conversion(error) => write!(f, "{error}"),
            WriteError::TooLarge => f.write_str("the boot information does not fit into a page"),
        }
    }
}

/// Fills `page` from the memory map in `map` and the ranges the loader
/// knows, and reports the framebuffer it actually described.
///
/// # Errors
///
/// [`WriteError`] for a memory map the region array cannot hold.
pub(crate) fn write(
    page: &mut [u8; BOOT_INFO_PAGE_LEN],
    map: &[u8],
    descriptor_size: usize,
    ranges: &Ranges,
    framebuffer: Option<Framebuffer>,
    wall_clock: Option<(i64, WallClockSource)>,
) -> Result<Option<Framebuffer>, WriteError> {
    let mut regions = [EMPTY_REGION; MAX_BOOT_REGIONS];
    let mut count = to_boot_regions(descriptors(map, descriptor_size), &mut regions)
        .map_err(WriteError::Conversion)?;
    let framebuffer = match framebuffer {
        Some(buffer) => describe_framebuffer(&mut regions, &mut count, buffer),
        None => None,
    };
    let used = regions.get(..count).unwrap_or(&[]);
    let rsdp = if used
        .iter()
        .any(|region| region.contains_range(ranges.rsdp, 1))
    {
        ranges.rsdp
    } else {
        0
    };
    let header = BootInfoHeader {
        phys_window_base: PHYS_WINDOW_BASE,
        kernel_phys_start: ranges.kernel.start,
        kernel_phys_len: ranges.kernel.len,
        boot_image_phys_start: ranges.boot_image.start,
        boot_image_phys_len: ranges.boot_image.len,
        page_tables_phys_start: ranges.page_tables.start,
        page_tables_phys_len: ranges.page_tables.len,
        boot_stack_phys_start: ranges.boot_stack.start,
        boot_stack_phys_len: ranges.boot_stack.len,
        acpi_rsdp: rsdp,
        wall_clock: wall_clock.map_or(0, |(_, source)| source.code()),
        boot_unix_seconds: wall_clock.map_or(0, |(seconds, _)| seconds),
        ..BootInfoHeader::default()
    };
    BootInfoWriter::write(page, &header, framebuffer, used).map_err(|_| WriteError::TooLarge)?;
    Ok(framebuffer)
}

/// The region a fresh array is filled with.
const EMPTY_REGION: BootRegion = BootRegion::new(0, 0, BootRegionKind::Reserved);

/// Makes sure a device region encloses the framebuffer, adding one if the
/// firmware reported none. A framebuffer the region array cannot describe
/// the way the kernel demands is dropped.
fn describe_framebuffer(
    regions: &mut [BootRegion; MAX_BOOT_REGIONS],
    count: &mut usize,
    framebuffer: Framebuffer,
) -> Option<Framebuffer> {
    let used = regions.get(..*count)?;
    if used.iter().any(|region| {
        region.kind == BootRegionKind::MmioReserved.code()
            && region.contains_range(framebuffer.phys_start, framebuffer.len)
    }) {
        return Some(framebuffer);
    }
    if used.iter().any(|region| {
        region.kind == BootRegionKind::Usable.code()
            && overlaps(*region, framebuffer.phys_start, framebuffer.len)
    }) {
        return None;
    }
    let device = BootRegion::new(
        framebuffer.phys_start,
        framebuffer.len,
        BootRegionKind::MmioReserved,
    );
    if insert_sorted(regions, count, device) {
        Some(framebuffer)
    } else {
        None
    }
}

/// Inserts `region` so that the array stays sorted by start address, and
/// reports whether there was room.
fn insert_sorted(
    regions: &mut [BootRegion; MAX_BOOT_REGIONS],
    count: &mut usize,
    region: BootRegion,
) -> bool {
    if *count >= MAX_BOOT_REGIONS {
        return false;
    }
    let mut position = *count;
    while position > 0 {
        let previous = position.saturating_sub(1);
        let Some(before) = regions.get(previous).copied() else {
            return false;
        };
        if before.start <= region.start {
            break;
        }
        let Some(slot) = regions.get_mut(position) else {
            return false;
        };
        *slot = before;
        position = previous;
    }
    let Some(slot) = regions.get_mut(position) else {
        return false;
    };
    *slot = region;
    *count = count.saturating_add(1);
    true
}

/// `true` if `start .. start + len` shares a byte with `region`.
const fn overlaps(region: BootRegion, start: u64, len: u64) -> bool {
    match (region.end(), start.checked_add(len)) {
        (Some(region_end), Some(end)) => {
            len != 0 && region.len != 0 && start < region_end && region.start < end
        }
        _ => false,
    }
}
