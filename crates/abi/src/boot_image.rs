// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The header of the boot image: the file the loader hands to the kernel.
//!
//! Invariants: a parsed [`BootImageHeader`] describes a root task and an
//! archive that lie inside the image, do not overlap, and start behind the
//! header; a serialized header always carries the current version, the
//! current header length, and zero flags.

use core::fmt;

use crate::layout::PAGE_SIZE;

/// The first eight bytes of every boot image.
pub const BOOT_IMAGE_MAGIC: [u8; 8] = *b"AUDHSOS\0";

/// The only format version this release understands.
pub const BOOT_IMAGE_VERSION: u32 = 1;

/// Length of the header in bytes.
pub const BOOT_IMAGE_HEADER_LEN: usize = 64;

/// [`BOOT_IMAGE_HEADER_LEN`] as a `u64`, for the offset arithmetic.
const HEADER_LEN: u64 = 64;

/// [`BOOT_IMAGE_HEADER_LEN`] as a `u32`, for the header-length field.
const HEADER_LEN_FIELD: u32 = 64;

const _: () = assert!(BOOT_IMAGE_HEADER_LEN == 64);

/// Number of `u32` words in the header.
const HEADER_WORDS: usize = BOOT_IMAGE_HEADER_LEN / 4;

/// The magic read as one little-endian `u64`.
const MAGIC_WORD: u64 = u64::from_le_bytes(BOOT_IMAGE_MAGIC);

/// The parsed header of a boot image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BootImageHeader {
    /// Offset of the root task inside the image.
    pub root_task_offset: u64,
    /// Length of the root task in bytes.
    pub root_task_len: u64,
    /// Offset of the archive inside the image.
    pub archive_offset: u64,
    /// Length of the archive in bytes; `0` for no archive.
    pub archive_len: u64,
    /// Requested size of the kernel reserve in bytes; `0` selects the
    /// default.
    pub kernel_reserve_size: u64,
}

/// Why a boot image header was rejected. The fields are validated in the
/// order in which they appear in the image; the cross-field checks come
/// last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BootImageError {
    /// Fewer than [`BOOT_IMAGE_HEADER_LEN`] bytes were available.
    TooShort,
    /// The magic is not [`BOOT_IMAGE_MAGIC`].
    BadMagic,
    /// The format version is not [`BOOT_IMAGE_VERSION`].
    UnsupportedVersion(u32),
    /// The header length is below the minimum or beyond the image.
    HeaderLength(u32),
    /// The root task offset is unaligned, inside the header, or beyond the
    /// image.
    RootTaskOffset,
    /// The root task length is zero or reaches beyond the image.
    RootTaskLength,
    /// The archive offset is unaligned, inside the header, or beyond the
    /// image.
    ArchiveOffset,
    /// The archive length reaches beyond the image.
    ArchiveLength,
    /// The archive and the root task share bytes.
    Overlap,
    /// The flags word is not zero.
    Flags(u64),
    /// The kernel reserve is unaligned or not below the usable memory.
    ReserveSize(u64),
}

impl fmt::Display for BootImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootImageError::TooShort => f.write_str("the boot image is shorter than its header"),
            BootImageError::BadMagic => f.write_str("the boot image magic does not match"),
            BootImageError::UnsupportedVersion(version) => {
                write!(f, "boot image version {version} is not supported")
            }
            BootImageError::HeaderLength(len) => {
                write!(f, "header length {len} is out of range")
            }
            BootImageError::RootTaskOffset => f.write_str("the root task offset is out of range"),
            BootImageError::RootTaskLength => f.write_str("the root task length is out of range"),
            BootImageError::ArchiveOffset => f.write_str("the archive offset is out of range"),
            BootImageError::ArchiveLength => f.write_str("the archive length is out of range"),
            BootImageError::Overlap => f.write_str("the archive overlaps the root task"),
            BootImageError::Flags(flags) => write!(f, "unknown boot image flags {flags:#x}"),
            BootImageError::ReserveSize(size) => {
                write!(f, "kernel reserve size {size} is out of range")
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

/// `true` if `value` is a multiple of the page size.
const fn page_aligned(value: u64) -> bool {
    value.is_multiple_of(PAGE_SIZE)
}

impl BootImageHeader {
    /// Parses and validates the header against `image_len`, the length of
    /// the whole image, and `ram_bytes`, the total usable memory.
    ///
    /// # Errors
    ///
    /// One [`BootImageError`] per rejected field; the fields are checked in
    /// the order in which they appear in the image.
    pub fn parse(bytes: &[u8], image_len: u64, ram_bytes: u64) -> Result<Self, BootImageError> {
        let header = bytes
            .get(..BOOT_IMAGE_HEADER_LEN)
            .ok_or(BootImageError::TooShort)?;
        let (chunks, _rest) = header.as_chunks::<4>();
        let mut words = [0u32; HEADER_WORDS];
        for (slot, chunk) in words.iter_mut().zip(chunks) {
            *slot = u32::from_le_bytes(*chunk);
        }
        let [
            magic_low,
            magic_high,
            version,
            header_len,
            root_task_offset_low,
            root_task_offset_high,
            root_task_len_low,
            root_task_len_high,
            archive_offset_low,
            archive_offset_high,
            archive_len_low,
            archive_len_high,
            kernel_reserve_low,
            kernel_reserve_high,
            flags_low,
            flags_high,
        ] = words;

        if join(magic_low, magic_high) != MAGIC_WORD {
            return Err(BootImageError::BadMagic);
        }
        if version != BOOT_IMAGE_VERSION {
            return Err(BootImageError::UnsupportedVersion(version));
        }
        let header_end = u64::from(header_len);
        if header_end < HEADER_LEN || header_end > image_len {
            return Err(BootImageError::HeaderLength(header_len));
        }

        let root_task_offset = join(root_task_offset_low, root_task_offset_high);
        if !page_aligned(root_task_offset)
            || root_task_offset < header_end
            || root_task_offset >= image_len
        {
            return Err(BootImageError::RootTaskOffset);
        }
        let root_task_len = join(root_task_len_low, root_task_len_high);
        let root_task_end = root_task_offset
            .checked_add(root_task_len)
            .ok_or(BootImageError::RootTaskLength)?;
        if root_task_len == 0 || root_task_end > image_len {
            return Err(BootImageError::RootTaskLength);
        }

        let archive_offset = join(archive_offset_low, archive_offset_high);
        if !page_aligned(archive_offset)
            || archive_offset < header_end
            || archive_offset > image_len
        {
            return Err(BootImageError::ArchiveOffset);
        }
        let archive_len = join(archive_len_low, archive_len_high);
        let archive_end = archive_offset
            .checked_add(archive_len)
            .ok_or(BootImageError::ArchiveLength)?;
        if archive_end > image_len {
            return Err(BootImageError::ArchiveLength);
        }

        let kernel_reserve_size = join(kernel_reserve_low, kernel_reserve_high);
        if kernel_reserve_size != 0
            && (!page_aligned(kernel_reserve_size) || kernel_reserve_size >= ram_bytes)
        {
            return Err(BootImageError::ReserveSize(kernel_reserve_size));
        }

        let flags = join(flags_low, flags_high);
        if flags != 0 {
            return Err(BootImageError::Flags(flags));
        }

        if archive_len != 0 && archive_offset < root_task_end && root_task_offset < archive_end {
            return Err(BootImageError::Overlap);
        }

        Ok(BootImageHeader {
            root_task_offset,
            root_task_len,
            archive_offset,
            archive_len,
            kernel_reserve_size,
        })
    }

    /// Serializes the header into its 64 little-endian bytes, with the
    /// current version, the current header length, and zero flags.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; BOOT_IMAGE_HEADER_LEN] {
        let fields: [u64; BOOT_IMAGE_HEADER_LEN / 8] = [
            MAGIC_WORD,
            join(BOOT_IMAGE_VERSION, HEADER_LEN_FIELD),
            self.root_task_offset,
            self.root_task_len,
            self.archive_offset,
            self.archive_len,
            self.kernel_reserve_size,
            0,
        ];
        let mut bytes = [0u8; BOOT_IMAGE_HEADER_LEN];
        let (chunks, _rest) = bytes.as_chunks_mut::<8>();
        for (chunk, value) in chunks.iter_mut().zip(fields) {
            *chunk = value.to_le_bytes();
        }
        bytes
    }
}
