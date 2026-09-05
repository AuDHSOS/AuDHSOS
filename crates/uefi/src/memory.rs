// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The memory map: the descriptor the firmware reports, the types it uses,
//! and the conversion into the boot regions the kernel reads.
//!
//! Invariants: the descriptors are read with the stride the firmware
//! reports, never with the size of the structure; the regions that come out
//! are sorted by start, non-empty, and pairwise disjoint.

use core::fmt;

use audhsos_abi::boot_info::{BootRegion, BootRegionKind};
use audhsos_abi::layout::{MAX_BOOT_REGIONS, PAGE_SIZE};

/// Length of the memory descriptor structure in bytes. The firmware may
/// report a larger stride; it never reports a smaller one.
pub const DESCRIPTOR_LEN: usize = 40;

/// Number of `u32` words in one descriptor.
const DESCRIPTOR_WORDS: usize = DESCRIPTOR_LEN / 4;

/// How the firmware should choose the address of an allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum AllocateType {
    /// Any address.
    AnyPages = 0,
    /// The highest free address below the one given.
    MaxAddress = 1,
    /// Exactly the address given.
    Address = 2,
}

/// What a memory region is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum MemoryType {
    /// Not to be used.
    Reserved = 0,
    /// Code of a loaded image.
    LoaderCode = 1,
    /// Data of a loaded image.
    LoaderData = 2,
    /// Code of the boot services.
    BootServicesCode = 3,
    /// Data of the boot services.
    BootServicesData = 4,
    /// Code of the runtime services.
    RuntimeServicesCode = 5,
    /// Data of the runtime services.
    RuntimeServicesData = 6,
    /// Free memory.
    Conventional = 7,
    /// Memory that reported an error.
    Unusable = 8,
    /// ACPI tables, free once they have been read.
    AcpiReclaim = 9,
    /// ACPI non-volatile storage.
    AcpiNvs = 10,
    /// Memory-mapped device registers.
    MappedIo = 11,
    /// Memory-mapped device ports.
    MappedIoPortSpace = 12,
    /// Processor firmware.
    PalCode = 13,
    /// Non-volatile memory that behaves like RAM.
    Persistent = 14,
    /// Memory the operating system has not accepted yet.
    Unaccepted = 15,
}

impl MemoryType {
    /// Every type the specification defines, in numeric order.
    pub const ALL: [MemoryType; 16] = [
        MemoryType::Reserved,
        MemoryType::LoaderCode,
        MemoryType::LoaderData,
        MemoryType::BootServicesCode,
        MemoryType::BootServicesData,
        MemoryType::RuntimeServicesCode,
        MemoryType::RuntimeServicesData,
        MemoryType::Conventional,
        MemoryType::Unusable,
        MemoryType::AcpiReclaim,
        MemoryType::AcpiNvs,
        MemoryType::MappedIo,
        MemoryType::MappedIoPortSpace,
        MemoryType::PalCode,
        MemoryType::Persistent,
        MemoryType::Unaccepted,
    ];

    /// The type with the given code, if the specification defines it.
    #[must_use]
    pub const fn from_u32(code: u32) -> Option<MemoryType> {
        match code {
            0 => Some(MemoryType::Reserved),
            1 => Some(MemoryType::LoaderCode),
            2 => Some(MemoryType::LoaderData),
            3 => Some(MemoryType::BootServicesCode),
            4 => Some(MemoryType::BootServicesData),
            5 => Some(MemoryType::RuntimeServicesCode),
            6 => Some(MemoryType::RuntimeServicesData),
            7 => Some(MemoryType::Conventional),
            8 => Some(MemoryType::Unusable),
            9 => Some(MemoryType::AcpiReclaim),
            10 => Some(MemoryType::AcpiNvs),
            11 => Some(MemoryType::MappedIo),
            12 => Some(MemoryType::MappedIoPortSpace),
            13 => Some(MemoryType::PalCode),
            14 => Some(MemoryType::Persistent),
            15 => Some(MemoryType::Unaccepted),
            _ => None,
        }
    }

    /// The numeric code.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            MemoryType::Reserved => 0,
            MemoryType::LoaderCode => 1,
            MemoryType::LoaderData => 2,
            MemoryType::BootServicesCode => 3,
            MemoryType::BootServicesData => 4,
            MemoryType::RuntimeServicesCode => 5,
            MemoryType::RuntimeServicesData => 6,
            MemoryType::Conventional => 7,
            MemoryType::Unusable => 8,
            MemoryType::AcpiReclaim => 9,
            MemoryType::AcpiNvs => 10,
            MemoryType::MappedIo => 11,
            MemoryType::MappedIoPortSpace => 12,
            MemoryType::PalCode => 13,
            MemoryType::Persistent => 14,
            MemoryType::Unaccepted => 15,
        }
    }

    /// The region kind the kernel reads for this type. The loader's own
    /// code and data are free once the loader is gone, so they become
    /// usable memory.
    #[must_use]
    pub const fn region_kind(self) -> BootRegionKind {
        match self {
            MemoryType::LoaderCode
            | MemoryType::LoaderData
            | MemoryType::BootServicesCode
            | MemoryType::BootServicesData
            | MemoryType::Conventional => BootRegionKind::Usable,
            MemoryType::AcpiReclaim => BootRegionKind::AcpiReclaimable,
            MemoryType::AcpiNvs => BootRegionKind::AcpiNvs,
            MemoryType::MappedIo | MemoryType::MappedIoPortSpace => BootRegionKind::MmioReserved,
            _ => BootRegionKind::Reserved,
        }
    }
}

/// One entry of the memory map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct MemoryDescriptor {
    /// The numeric code of a [`MemoryType`].
    pub type_: u32,
    /// First byte of the region.
    pub physical_start: u64,
    /// Where the region will live after the virtual address map is set.
    pub virtual_start: u64,
    /// Length of the region in pages.
    pub pages: u64,
    /// The capabilities of the region.
    pub attribute: u64,
}

impl MemoryDescriptor {
    /// Decodes one descriptor from the first [`DESCRIPTOR_LEN`] bytes of
    /// `bytes`, or `None` if fewer are available.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<MemoryDescriptor> {
        let fixed = bytes.get(..DESCRIPTOR_LEN)?;
        let (chunks, _rest) = fixed.as_chunks::<4>();
        let mut words = [0u32; DESCRIPTOR_WORDS];
        for (slot, chunk) in words.iter_mut().zip(chunks) {
            *slot = u32::from_le_bytes(*chunk);
        }
        let [
            type_,
            _pad,
            start_low,
            start_high,
            virtual_low,
            virtual_high,
            pages_low,
            pages_high,
            attribute_low,
            attribute_high,
        ] = words;
        Some(MemoryDescriptor {
            type_,
            physical_start: join(start_low, start_high),
            virtual_start: join(virtual_low, virtual_high),
            pages: join(pages_low, pages_high),
            attribute: join(attribute_low, attribute_high),
        })
    }

    /// The length of the region in bytes, if it is representable.
    #[must_use]
    pub const fn bytes(self) -> Option<u64> {
        self.pages.checked_mul(PAGE_SIZE)
    }

    /// The first byte above the region, if it is representable.
    #[must_use]
    pub const fn end(self) -> Option<u64> {
        match self.bytes() {
            Some(len) => self.physical_start.checked_add(len),
            None => None,
        }
    }

    /// The region kind the kernel reads; an unknown type is reserved.
    #[must_use]
    pub const fn region_kind(self) -> BootRegionKind {
        match MemoryType::from_u32(self.type_) {
            Some(kind) => kind.region_kind(),
            None => BootRegionKind::Reserved,
        }
    }
}

/// The two halves of a `u64` field, low word first, joined again.
#[expect(
    clippy::as_conversions,
    reason = "widening two u32 halves into a u64, in a const fn"
)]
const fn join(low: u32, high: u32) -> u64 {
    ((high as u64) << 32) | (low as u64)
}

/// Why a memory map could not be converted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConversionError {
    /// More regions than the boot information structure holds.
    TooManyRegions,
    /// A descriptor reaches beyond the representable addresses.
    Overflow,
    /// Two descriptors of different kinds share memory.
    Overlap,
}

impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConversionError::TooManyRegions => f.write_str("the memory map has too many regions"),
            ConversionError::Overflow => {
                f.write_str("a descriptor reaches beyond the address space")
            }
            ConversionError::Overlap => {
                f.write_str("two descriptors of different kinds share memory")
            }
        }
    }
}

/// The descriptors in `buffer`, read with the stride the firmware
/// reported. A stride below [`DESCRIPTOR_LEN`] yields nothing, because the
/// buffer cannot hold descriptors at all.
pub fn descriptors(
    buffer: &[u8],
    descriptor_size: usize,
) -> impl Iterator<Item = MemoryDescriptor> {
    let usable = descriptor_size >= DESCRIPTOR_LEN;
    let stride = if usable {
        descriptor_size
    } else {
        DESCRIPTOR_LEN
    };
    buffer
        .chunks(stride)
        .take_while(move |_| usable)
        .filter_map(MemoryDescriptor::decode)
}

/// Turns the memory map into the region array of the boot information
/// structure: empty descriptors dropped, the rest sorted by start and
/// merged where they touch or overlap and agree on the kind. Regions of
/// different kinds may touch; only shared bytes are an error. Returns the
/// number of regions written.
///
/// # Errors
///
/// [`ConversionError::Overflow`] if a descriptor reaches beyond the
/// address space; [`ConversionError::Overlap`] if two descriptors of
/// different kinds share memory; [`ConversionError::TooManyRegions`] if
/// more than [`MAX_BOOT_REGIONS`] regions remain.
pub fn to_boot_regions<I: Iterator<Item = MemoryDescriptor>>(
    descriptors: I,
    out: &mut [BootRegion; MAX_BOOT_REGIONS],
) -> Result<usize, ConversionError> {
    let mut count = 0usize;
    for descriptor in descriptors {
        if descriptor.pages == 0 {
            continue;
        }
        let len = descriptor.bytes().ok_or(ConversionError::Overflow)?;
        descriptor.end().ok_or(ConversionError::Overflow)?;
        let slot = out.get_mut(count).ok_or(ConversionError::TooManyRegions)?;
        *slot = BootRegion {
            start: descriptor.physical_start,
            len,
            kind: descriptor.region_kind().code(),
            reserved: 0,
        };
        count = count.saturating_add(1);
    }
    sort_by_start(out, count);
    merge(out, count)
}

/// Insertion sort by start address.
fn sort_by_start(regions: &mut [BootRegion; MAX_BOOT_REGIONS], count: usize) {
    let mut index = 1;
    while index < count {
        let mut position = index;
        while position > 0 {
            let previous = position.saturating_sub(1);
            match (
                regions.get(previous).copied(),
                regions.get(position).copied(),
            ) {
                (Some(left), Some(right)) if left.start > right.start => {
                    put(regions, previous, right);
                    put(regions, position, left);
                }
                _ => break,
            }
            position = previous;
        }
        index = index.saturating_add(1);
    }
}

/// Merges regions that touch or overlap and agree on the kind.
fn merge(
    regions: &mut [BootRegion; MAX_BOOT_REGIONS],
    count: usize,
) -> Result<usize, ConversionError> {
    let mut written = 0usize;
    let mut index = 0usize;
    let mut current: Option<BootRegion> = None;
    while index < count {
        let Some(next) = regions.get(index).copied() else {
            break;
        };
        index = index.saturating_add(1);
        let Some(open) = current else {
            current = Some(next);
            continue;
        };
        let open_end = open.end().ok_or(ConversionError::Overflow)?;
        let touches = next.start == open_end;
        if next.start > open_end || (touches && next.kind != open.kind) {
            put(regions, written, open);
            written = written.saturating_add(1);
            current = Some(next);
            continue;
        }
        if next.kind != open.kind {
            return Err(ConversionError::Overlap);
        }
        let next_end = next.end().ok_or(ConversionError::Overflow)?;
        let end = open_end.max(next_end);
        current = Some(BootRegion {
            start: open.start,
            len: end.saturating_sub(open.start),
            kind: open.kind,
            reserved: 0,
        });
    }
    if let Some(open) = current {
        put(regions, written, open);
        written = written.saturating_add(1);
    }
    let mut tail = written;
    while tail < count {
        put(
            regions,
            tail,
            BootRegion::new(0, 0, BootRegionKind::Reserved),
        );
        tail = tail.saturating_add(1);
    }
    Ok(written)
}

/// Writes `region` into slot `index`; an index outside the array changes
/// nothing.
pub(crate) fn put(regions: &mut [BootRegion; MAX_BOOT_REGIONS], index: usize, region: BootRegion) {
    if let Some(slot) = regions.get_mut(index) {
        *slot = region;
    }
}
