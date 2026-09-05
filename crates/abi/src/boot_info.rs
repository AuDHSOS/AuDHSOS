// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The boot information structure: what the loader tells the kernel about
//! the machine.
//!
//! Invariants: a parsed [`BootInfoView`] describes at most
//! [`MAX_BOOT_REGIONS`] regions, each of a known kind and of non-zero
//! length; the four fixed ranges are pairwise disjoint and each lies inside
//! one region; the whole structure fits into one page.
//!
//! The structure is decoded field by field instead of being reinterpreted
//! in place, so that parsing needs no `unsafe` and can be fuzzed on the
//! host.

use core::fmt;

use crate::layout::{MAX_BOOT_REGIONS, PAGE_SIZE};

/// The first eight bytes of every boot information structure.
pub const BOOT_INFO_MAGIC: [u8; 8] = *b"AUDHBOOT";

/// The only version this release understands.
pub const BOOT_INFO_VERSION: u32 = 1;

/// Length of the fixed part in bytes.
pub const BOOT_INFO_HEADER_LEN: usize = 136;

/// Length of one region entry in bytes.
pub const BOOT_REGION_LEN: usize = 24;

/// Length of the page the loader writes the structure into.
pub const BOOT_INFO_PAGE_LEN: usize = 4096;

const _: () = assert!(PAGE_SIZE == 4096);
const _: () = assert!(BOOT_INFO_PAGE_LEN == 4096);
const _: () =
    assert!(BOOT_INFO_HEADER_LEN + MAX_BOOT_REGIONS * BOOT_REGION_LEN <= BOOT_INFO_PAGE_LEN);

/// [`BOOT_INFO_HEADER_LEN`] as a `u32`, for the size arithmetic.
const HEADER_LEN_FIELD: u32 = 136;

/// [`BOOT_REGION_LEN`] as a `u32`, for the size arithmetic.
const REGION_LEN_FIELD: u32 = 24;

/// [`BOOT_INFO_PAGE_LEN`] as a `u32`, for the size arithmetic.
const PAGE_LEN_FIELD: u32 = 4096;

/// [`MAX_BOOT_REGIONS`] as a `u32`, for the region-count check.
const MAX_REGIONS_FIELD: u32 = 128;

const _: () = assert!(MAX_BOOT_REGIONS == 128);

/// Number of `u32` words in the fixed part.
const HEADER_WORDS: usize = BOOT_INFO_HEADER_LEN / 4;

/// Number of `u32` words in one region entry.
const REGION_WORDS: usize = BOOT_REGION_LEN / 4;

/// The magic read as one little-endian `u64`.
const MAGIC_WORD: u64 = u64::from_le_bytes(BOOT_INFO_MAGIC);

/// What a physical region holds, as reported by the firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum BootRegionKind {
    /// Free memory.
    Usable = 1,
    /// Not usable by the kernel.
    Reserved = 2,
    /// ACPI tables, usable once they have been read.
    AcpiReclaimable = 3,
    /// ACPI non-volatile storage, never usable.
    AcpiNvs = 4,
    /// Memory-mapped device registers.
    MmioReserved = 5,
}

impl BootRegionKind {
    /// Every kind, in numeric order.
    pub const ALL: &'static [BootRegionKind] = &[
        BootRegionKind::Usable,
        BootRegionKind::Reserved,
        BootRegionKind::AcpiReclaimable,
        BootRegionKind::AcpiNvs,
        BootRegionKind::MmioReserved,
    ];

    /// The stable numeric code.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            BootRegionKind::Usable => 1,
            BootRegionKind::Reserved => 2,
            BootRegionKind::AcpiReclaimable => 3,
            BootRegionKind::AcpiNvs => 4,
            BootRegionKind::MmioReserved => 5,
        }
    }

    /// The kind with the given code, if it is known.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(BootRegionKind::Usable),
            2 => Some(BootRegionKind::Reserved),
            3 => Some(BootRegionKind::AcpiReclaimable),
            4 => Some(BootRegionKind::AcpiNvs),
            5 => Some(BootRegionKind::MmioReserved),
            _ => None,
        }
    }
}

/// One physical memory region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct BootRegion {
    /// First byte of the region.
    pub start: u64,
    /// Length in bytes.
    pub len: u64,
    /// The numeric code of a [`BootRegionKind`].
    pub kind: u32,
    /// Zero in version 1.
    pub reserved: u32,
}

impl BootRegion {
    /// A region of `len` bytes at `start` of the given kind.
    #[must_use]
    pub const fn new(start: u64, len: u64, kind: BootRegionKind) -> Self {
        BootRegion {
            start,
            len,
            kind: kind.code(),
            reserved: 0,
        }
    }

    /// The first byte above the region, if it is representable.
    #[must_use]
    pub const fn end(self) -> Option<u64> {
        self.start.checked_add(self.len)
    }

    /// `true` if `start..start + len` lies completely inside the region.
    #[must_use]
    pub const fn contains_range(self, start: u64, len: u64) -> bool {
        match (self.end(), start.checked_add(len)) {
            (Some(region_end), Some(end)) => start >= self.start && end <= region_end,
            _ => false,
        }
    }
}

/// The fixed part of the boot information structure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct BootInfoHeader {
    /// [`BOOT_INFO_MAGIC`].
    pub magic: [u8; 8],
    /// [`BOOT_INFO_VERSION`].
    pub version: u32,
    /// Total size including the region array.
    pub size: u32,
    /// Virtual base of the physical memory window.
    pub phys_window_base: u64,
    /// First byte of the kernel image.
    pub kernel_phys_start: u64,
    /// Length of the kernel image.
    pub kernel_phys_len: u64,
    /// First byte of the boot image.
    pub boot_image_phys_start: u64,
    /// Length of the boot image.
    pub boot_image_phys_len: u64,
    /// First byte of the initial page tables.
    pub page_tables_phys_start: u64,
    /// Length of the initial page tables.
    pub page_tables_phys_len: u64,
    /// First byte of the boot stack, guard page excluded.
    pub boot_stack_phys_start: u64,
    /// Length of the boot stack.
    pub boot_stack_phys_len: u64,
    /// Physical address of the ACPI root pointer; `0` if absent.
    pub acpi_rsdp: u64,
    /// Physical base of the linear framebuffer; `0` if absent.
    pub framebuffer_phys_start: u64,
    /// Length of the framebuffer in bytes; `0` if absent.
    pub framebuffer_len: u64,
    /// Visible pixels per row.
    pub framebuffer_width: u32,
    /// Visible rows.
    pub framebuffer_height: u32,
    /// Pixels per scan line.
    pub framebuffer_stride: u32,
    /// The code of a [`FramebufferFormat`]; `0` if absent.
    pub framebuffer_format: u32,
    /// Number of entries in the region array.
    pub region_count: u32,
    /// Zero in version 1.
    pub reserved: u32,
}

/// How the bytes of a framebuffer pixel are ordered. Both formats carry
/// four bytes per pixel with an unused fourth byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum FramebufferFormat {
    /// Red in the lowest byte.
    Rgbx8888 = 1,
    /// Blue in the lowest byte.
    Bgrx8888 = 2,
}

impl FramebufferFormat {
    /// Every format, in numeric order.
    pub const ALL: &'static [FramebufferFormat] =
        &[FramebufferFormat::Rgbx8888, FramebufferFormat::Bgrx8888];

    /// The stable numeric code; `0` means that no framebuffer is present.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            FramebufferFormat::Rgbx8888 => 1,
            FramebufferFormat::Bgrx8888 => 2,
        }
    }

    /// The format with the given code, if it is known.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(FramebufferFormat::Rgbx8888),
            2 => Some(FramebufferFormat::Bgrx8888),
            _ => None,
        }
    }
}

/// Number of bytes one pixel occupies in every format.
pub const BYTES_PER_PIXEL: u64 = 4;

/// The linear framebuffer the firmware set up, as the loader found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Framebuffer {
    /// Physical base of the framebuffer, frame-aligned.
    pub phys_start: u64,
    /// Length in bytes, a multiple of the frame size.
    pub len: u64,
    /// Visible pixels per row.
    pub width: u32,
    /// Visible rows.
    pub height: u32,
    /// Pixels per scan line, at least `width`.
    pub stride: u32,
    /// The pixel format.
    pub format: FramebufferFormat,
}

impl Framebuffer {
    /// The number of bytes the visible pixels occupy, if it is
    /// representable.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "widening two u32 dimensions into a u64, in a const fn"
    )]
    pub const fn visible_bytes(self) -> Option<u64> {
        match (self.height as u64).checked_mul(self.stride as u64) {
            Some(pixels) => pixels.checked_mul(BYTES_PER_PIXEL),
            None => None,
        }
    }

    /// The first byte above the framebuffer, if it is representable.
    #[must_use]
    pub const fn end(self) -> Option<u64> {
        self.phys_start.checked_add(self.len)
    }
}

/// One of the four ranges the loader reports separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FixedRange {
    /// The kernel image.
    Kernel,
    /// The boot image.
    BootImage,
    /// The initial page tables.
    PageTables,
    /// The boot stack.
    BootStack,
}

impl FixedRange {
    /// Every range, in the order of the header fields.
    pub const ALL: &'static [FixedRange] = &[
        FixedRange::Kernel,
        FixedRange::BootImage,
        FixedRange::PageTables,
        FixedRange::BootStack,
    ];

    /// The name used in messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            FixedRange::Kernel => "kernel image",
            FixedRange::BootImage => "boot image",
            FixedRange::PageTables => "page tables",
            FixedRange::BootStack => "boot stack",
        }
    }
}

/// Why a boot information structure was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BootInfoError {
    /// Fewer bytes were available than the structure claims to need.
    TooShort,
    /// The magic is not [`BOOT_INFO_MAGIC`].
    BadMagic,
    /// The version is not [`BOOT_INFO_VERSION`].
    UnsupportedVersion(u32),
    /// The size does not match the region count or exceeds one page.
    Size(u32),
    /// The region count is above [`MAX_BOOT_REGIONS`].
    RegionCount(u32),
    /// A region carries a kind this version does not know.
    RegionKind {
        /// Index in the region array.
        index: usize,
        /// The unknown code.
        kind: u32,
    },
    /// A region has zero length.
    RegionLength(usize),
    /// A region reaches beyond the representable addresses.
    RegionOverflow(usize),
    /// A fixed range has zero length.
    RangeEmpty(FixedRange),
    /// A fixed range reaches beyond the representable addresses.
    RangeOverflow(FixedRange),
    /// Two fixed ranges share bytes.
    RangeOverlap(FixedRange, FixedRange),
    /// A fixed range lies outside every reported region.
    RangeOutsideRegions(FixedRange),
    /// The ACPI root pointer lies outside every reported region.
    AcpiPointer(u64),
    /// The framebuffer format code is not one this version knows.
    FramebufferFormat(u32),
    /// A framebuffer field is set although no framebuffer is reported.
    FramebufferAbsentFieldSet,
    /// The framebuffer base is zero or not frame-aligned.
    FramebufferBase(u64),
    /// A framebuffer dimension is zero, or the stride is below the width.
    FramebufferResolution,
    /// The framebuffer is shorter than its visible pixels, or its length
    /// is not a multiple of the frame size.
    FramebufferLength(u64),
    /// The size of the framebuffer is not representable.
    FramebufferOverflow,
    /// The framebuffer shares memory with a usable region.
    FramebufferOverlapsUsable,
    /// No reported device region encloses the framebuffer.
    FramebufferNotReserved,
}

impl fmt::Display for BootInfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootInfoError::TooShort => f.write_str("the boot information is truncated"),
            BootInfoError::BadMagic => f.write_str("the boot information magic does not match"),
            BootInfoError::UnsupportedVersion(version) => {
                write!(f, "boot information version {version} is not supported")
            }
            BootInfoError::Size(size) => write!(f, "boot information size {size} is out of range"),
            BootInfoError::RegionCount(count) => write!(f, "region count {count} is too large"),
            BootInfoError::RegionKind { index, kind } => {
                write!(f, "region {index} has the unknown kind {kind}")
            }
            BootInfoError::RegionLength(index) => write!(f, "region {index} is empty"),
            BootInfoError::RegionOverflow(index) => {
                write!(f, "region {index} reaches beyond the address space")
            }
            BootInfoError::RangeEmpty(range) => write!(f, "the {} range is empty", range.name()),
            BootInfoError::RangeOverflow(range) => write!(
                f,
                "the {} range reaches beyond the address space",
                range.name()
            ),
            BootInfoError::RangeOverlap(first, second) => write!(
                f,
                "the {} range overlaps the {} range",
                first.name(),
                second.name()
            ),
            BootInfoError::RangeOutsideRegions(range) => write!(
                f,
                "the {} range lies outside every reported region",
                range.name()
            ),
            BootInfoError::AcpiPointer(address) => write!(
                f,
                "the ACPI root pointer {address:#x} lies outside every reported region"
            ),
            BootInfoError::FramebufferFormat(code) => {
                write!(f, "framebuffer format {code} is not supported")
            }
            BootInfoError::FramebufferAbsentFieldSet => {
                f.write_str("a framebuffer field is set although no framebuffer is reported")
            }
            BootInfoError::FramebufferBase(base) => {
                write!(f, "the framebuffer base {base:#x} is not usable")
            }
            BootInfoError::FramebufferResolution => {
                f.write_str("the framebuffer resolution is not usable")
            }
            BootInfoError::FramebufferLength(len) => {
                write!(f, "the framebuffer length {len} is not usable")
            }
            BootInfoError::FramebufferOverflow => {
                f.write_str("the size of the framebuffer is not representable")
            }
            BootInfoError::FramebufferOverlapsUsable => {
                f.write_str("the framebuffer shares memory with usable memory")
            }
            BootInfoError::FramebufferNotReserved => {
                f.write_str("no reported device region encloses the framebuffer")
            }
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

/// The `u32` words of `bytes`, as many as fit into `N`.
fn words<const N: usize>(bytes: &[u8]) -> [u32; N] {
    let mut result = [0u32; N];
    let (chunks, _rest) = bytes.as_chunks::<4>();
    for (slot, chunk) in result.iter_mut().zip(chunks) {
        *slot = u32::from_le_bytes(*chunk);
    }
    result
}

/// A validated, borrowed view of a boot information structure.
#[derive(Clone, Copy, Debug)]
pub struct BootInfoView<'a> {
    header: BootInfoHeader,
    regions: &'a [u8],
}

impl<'a> BootInfoView<'a> {
    /// Decodes and validates the structure at the start of `bytes`.
    ///
    /// # Errors
    ///
    /// One [`BootInfoError`] per rejected field.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, BootInfoError> {
        let fixed = bytes
            .get(..BOOT_INFO_HEADER_LEN)
            .ok_or(BootInfoError::TooShort)?;
        let [
            magic_low,
            magic_high,
            version,
            size,
            phys_window_low,
            phys_window_high,
            kernel_start_low,
            kernel_start_high,
            kernel_len_low,
            kernel_len_high,
            image_start_low,
            image_start_high,
            image_len_low,
            image_len_high,
            tables_start_low,
            tables_start_high,
            tables_len_low,
            tables_len_high,
            stack_start_low,
            stack_start_high,
            stack_len_low,
            stack_len_high,
            rsdp_low,
            rsdp_high,
            framebuffer_start_low,
            framebuffer_start_high,
            framebuffer_len_low,
            framebuffer_len_high,
            framebuffer_width,
            framebuffer_height,
            framebuffer_stride,
            framebuffer_format,
            region_count,
            reserved,
        ] = words::<HEADER_WORDS>(fixed);

        if join(magic_low, magic_high) != MAGIC_WORD {
            return Err(BootInfoError::BadMagic);
        }
        if version != BOOT_INFO_VERSION {
            return Err(BootInfoError::UnsupportedVersion(version));
        }
        if !(HEADER_LEN_FIELD..=PAGE_LEN_FIELD).contains(&size) {
            return Err(BootInfoError::Size(size));
        }
        if region_count > MAX_REGIONS_FIELD {
            return Err(BootInfoError::RegionCount(region_count));
        }
        let expected =
            HEADER_LEN_FIELD.saturating_add(region_count.saturating_mul(REGION_LEN_FIELD));
        if size != expected {
            return Err(BootInfoError::Size(size));
        }
        let end = usize::try_from(expected).unwrap_or(usize::MAX);
        let regions = bytes
            .get(BOOT_INFO_HEADER_LEN..end)
            .ok_or(BootInfoError::TooShort)?;

        let header = BootInfoHeader {
            magic: BOOT_INFO_MAGIC,
            version,
            size,
            phys_window_base: join(phys_window_low, phys_window_high),
            kernel_phys_start: join(kernel_start_low, kernel_start_high),
            kernel_phys_len: join(kernel_len_low, kernel_len_high),
            boot_image_phys_start: join(image_start_low, image_start_high),
            boot_image_phys_len: join(image_len_low, image_len_high),
            page_tables_phys_start: join(tables_start_low, tables_start_high),
            page_tables_phys_len: join(tables_len_low, tables_len_high),
            boot_stack_phys_start: join(stack_start_low, stack_start_high),
            boot_stack_phys_len: join(stack_len_low, stack_len_high),
            acpi_rsdp: join(rsdp_low, rsdp_high),
            framebuffer_phys_start: join(framebuffer_start_low, framebuffer_start_high),
            framebuffer_len: join(framebuffer_len_low, framebuffer_len_high),
            framebuffer_width,
            framebuffer_height,
            framebuffer_stride,
            framebuffer_format,
            region_count,
            reserved,
        };
        let view = BootInfoView { header, regions };
        view.check_regions()?;
        view.check_fixed_ranges()?;
        view.check_acpi_pointer()?;
        view.check_framebuffer()?;
        Ok(view)
    }

    fn check_regions(&self) -> Result<(), BootInfoError> {
        for (index, region) in self.regions().enumerate() {
            if BootRegionKind::from_code(region.kind).is_none() {
                return Err(BootInfoError::RegionKind {
                    index,
                    kind: region.kind,
                });
            }
            if region.len == 0 {
                return Err(BootInfoError::RegionLength(index));
            }
            if region.end().is_none() {
                return Err(BootInfoError::RegionOverflow(index));
            }
        }
        Ok(())
    }

    fn check_fixed_ranges(&self) -> Result<(), BootInfoError> {
        for (position, &name) in FixedRange::ALL.iter().enumerate() {
            let (start, len) = self.fixed_range(name);
            if len == 0 {
                return Err(BootInfoError::RangeEmpty(name));
            }
            let end = start
                .checked_add(len)
                .ok_or(BootInfoError::RangeOverflow(name))?;
            if !self
                .regions()
                .any(|region| region.contains_range(start, len))
            {
                return Err(BootInfoError::RangeOutsideRegions(name));
            }
            for &other in FixedRange::ALL.iter().skip(position.saturating_add(1)) {
                let (other_start, other_len) = self.fixed_range(other);
                let other_end = other_start
                    .checked_add(other_len)
                    .ok_or(BootInfoError::RangeOverflow(other))?;
                if other_len != 0 && other_start < end && start < other_end {
                    return Err(BootInfoError::RangeOverlap(name, other));
                }
            }
        }
        Ok(())
    }

    fn check_acpi_pointer(&self) -> Result<(), BootInfoError> {
        let rsdp = self.header.acpi_rsdp;
        if rsdp == 0 || self.regions().any(|region| region.contains_range(rsdp, 1)) {
            return Ok(());
        }
        Err(BootInfoError::AcpiPointer(rsdp))
    }

    /// The framebuffer the loader found, or `None` when the machine has
    /// none. Only a structure that came through [`BootInfoView::parse`]
    /// reports one, so every field of it has been checked.
    #[must_use]
    pub const fn framebuffer(&self) -> Option<Framebuffer> {
        match FramebufferFormat::from_code(self.header.framebuffer_format) {
            Some(format) => Some(Framebuffer {
                phys_start: self.header.framebuffer_phys_start,
                len: self.header.framebuffer_len,
                width: self.header.framebuffer_width,
                height: self.header.framebuffer_height,
                stride: self.header.framebuffer_stride,
                format,
            }),
            None => None,
        }
    }

    /// Rejects a framebuffer description the loader should never write.
    fn check_framebuffer(&self) -> Result<(), BootInfoError> {
        let header = &self.header;
        if header.framebuffer_format == 0 {
            let absent = header.framebuffer_phys_start == 0
                && header.framebuffer_len == 0
                && header.framebuffer_width == 0
                && header.framebuffer_height == 0
                && header.framebuffer_stride == 0;
            return if absent {
                Ok(())
            } else {
                Err(BootInfoError::FramebufferAbsentFieldSet)
            };
        }
        let Some(framebuffer) = self.framebuffer() else {
            return Err(BootInfoError::FramebufferFormat(header.framebuffer_format));
        };
        if framebuffer.phys_start == 0 || !framebuffer.phys_start.is_multiple_of(PAGE_SIZE) {
            return Err(BootInfoError::FramebufferBase(framebuffer.phys_start));
        }
        if framebuffer.width == 0
            || framebuffer.height == 0
            || framebuffer.stride < framebuffer.width
        {
            return Err(BootInfoError::FramebufferResolution);
        }
        let visible = framebuffer
            .visible_bytes()
            .ok_or(BootInfoError::FramebufferOverflow)?;
        if framebuffer.len < visible || !framebuffer.len.is_multiple_of(PAGE_SIZE) {
            return Err(BootInfoError::FramebufferLength(framebuffer.len));
        }
        framebuffer
            .end()
            .ok_or(BootInfoError::FramebufferOverflow)?;
        if self.regions().any(|region| {
            region.kind == BootRegionKind::Usable.code()
                && overlaps(region, framebuffer.phys_start, framebuffer.len)
        }) {
            return Err(BootInfoError::FramebufferOverlapsUsable);
        }
        if !self.regions().any(|region| {
            region.kind == BootRegionKind::MmioReserved.code()
                && region.contains_range(framebuffer.phys_start, framebuffer.len)
        }) {
            return Err(BootInfoError::FramebufferNotReserved);
        }
        Ok(())
    }

    /// The start and the length of one of the four fixed ranges.
    #[must_use]
    pub const fn fixed_range(&self, range: FixedRange) -> (u64, u64) {
        match range {
            FixedRange::Kernel => (self.header.kernel_phys_start, self.header.kernel_phys_len),
            FixedRange::BootImage => (
                self.header.boot_image_phys_start,
                self.header.boot_image_phys_len,
            ),
            FixedRange::PageTables => (
                self.header.page_tables_phys_start,
                self.header.page_tables_phys_len,
            ),
            FixedRange::BootStack => (
                self.header.boot_stack_phys_start,
                self.header.boot_stack_phys_len,
            ),
        }
    }

    /// The fixed part of the structure.
    #[must_use]
    pub const fn header(&self) -> BootInfoHeader {
        self.header
    }

    /// The number of regions.
    #[must_use]
    pub const fn region_count(&self) -> usize {
        self.regions.len().wrapping_div(BOOT_REGION_LEN)
    }

    /// The regions, in the order the loader wrote them.
    pub fn regions(&self) -> impl Iterator<Item = BootRegion> + '_ {
        let (chunks, _rest) = self.regions.as_chunks::<BOOT_REGION_LEN>();
        chunks.iter().map(|chunk| {
            let [start_low, start_high, len_low, len_high, kind, reserved] =
                words::<REGION_WORDS>(chunk);
            BootRegion {
                start: join(start_low, start_high),
                len: join(len_low, len_high),
                kind,
                reserved,
            }
        })
    }

    /// Virtual base of the physical memory window.
    #[must_use]
    pub const fn phys_window_base(&self) -> u64 {
        self.header.phys_window_base
    }

    /// Physical address of the ACPI root pointer, if the firmware provided
    /// one.
    #[must_use]
    pub const fn acpi_rsdp(&self) -> Option<u64> {
        if self.header.acpi_rsdp == 0 {
            None
        } else {
            Some(self.header.acpi_rsdp)
        }
    }
}

/// Writes a boot information structure into the page the loader reserved
/// for it.
#[derive(Clone, Copy, Debug)]
pub struct BootInfoWriter;

impl BootInfoWriter {
    /// Fills `page` from `header`, `framebuffer`, and `regions` and returns
    /// the number of bytes written. The magic, the version, the size, and
    /// the region count come from `regions`, and the framebuffer fields
    /// from `framebuffer`, not from `header`; a writer and a parser that
    /// disagree would be a defect the tests could not see.
    ///
    /// # Errors
    ///
    /// [`BootInfoError::RegionCount`] if `regions` holds more than
    /// [`MAX_BOOT_REGIONS`] entries.
    pub fn write(
        page: &mut [u8; BOOT_INFO_PAGE_LEN],
        header: &BootInfoHeader,
        framebuffer: Option<Framebuffer>,
        regions: &[BootRegion],
    ) -> Result<u32, BootInfoError> {
        let count = u32::try_from(regions.len()).unwrap_or(u32::MAX);
        if count > MAX_REGIONS_FIELD {
            return Err(BootInfoError::RegionCount(count));
        }
        let size = HEADER_LEN_FIELD.saturating_add(count.saturating_mul(REGION_LEN_FIELD));
        let fixed: [u32; HEADER_WORDS] = [
            low(MAGIC_WORD),
            high(MAGIC_WORD),
            BOOT_INFO_VERSION,
            size,
            low(header.phys_window_base),
            high(header.phys_window_base),
            low(header.kernel_phys_start),
            high(header.kernel_phys_start),
            low(header.kernel_phys_len),
            high(header.kernel_phys_len),
            low(header.boot_image_phys_start),
            high(header.boot_image_phys_start),
            low(header.boot_image_phys_len),
            high(header.boot_image_phys_len),
            low(header.page_tables_phys_start),
            high(header.page_tables_phys_start),
            low(header.page_tables_phys_len),
            high(header.page_tables_phys_len),
            low(header.boot_stack_phys_start),
            high(header.boot_stack_phys_start),
            low(header.boot_stack_phys_len),
            high(header.boot_stack_phys_len),
            low(header.acpi_rsdp),
            high(header.acpi_rsdp),
            low(framebuffer.map_or(0, |buffer| buffer.phys_start)),
            high(framebuffer.map_or(0, |buffer| buffer.phys_start)),
            low(framebuffer.map_or(0, |buffer| buffer.len)),
            high(framebuffer.map_or(0, |buffer| buffer.len)),
            framebuffer.map_or(0, |buffer| buffer.width),
            framebuffer.map_or(0, |buffer| buffer.height),
            framebuffer.map_or(0, |buffer| buffer.stride),
            framebuffer.map_or(0, |buffer| buffer.format.code()),
            count,
            0,
        ];
        page.fill(0);
        let (chunks, _rest) = page.as_chunks_mut::<4>();
        let values = fixed.into_iter().chain(regions.iter().flat_map(|region| {
            [
                low(region.start),
                high(region.start),
                low(region.len),
                high(region.len),
                region.kind,
                region.reserved,
            ]
        }));
        for (slot, value) in chunks.iter_mut().zip(values) {
            *slot = value.to_le_bytes();
        }
        Ok(size)
    }
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

/// The low half of a `u64`.
#[expect(
    clippy::as_conversions,
    reason = "narrowing a value masked to 32 bits, in a const fn"
)]
const fn low(value: u64) -> u32 {
    (value & 0xFFFF_FFFF) as u32
}

/// The high half of a `u64`.
#[expect(
    clippy::as_conversions,
    reason = "narrowing the upper 32 bits of a u64, in a const fn"
)]
const fn high(value: u64) -> u32 {
    (value >> 32) as u32
}
