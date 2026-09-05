// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The section header table, which the loader does not read and a
//! symbolizer does.
//!
//! Invariants: a section this module hands out lies inside the file, or it
//! is not handed out; a section of type `SHT_NOBITS` occupies no file
//! range whatever its size says.

use crate::error::ElfError;
use crate::image::{EHDR_LEN, words};

/// Number of bytes of one section header entry.
pub const SHDR_LEN: usize = 64;

/// Number of `u32` words in a section header entry.
const SHDR_WORDS: usize = SHDR_LEN / 4;

/// Offset of the section header table offset in the file header.
const SHOFF_OFFSET: usize = 0x28;

/// Offset of the section header entry size in the file header.
const SHENTSIZE_OFFSET: usize = 0x3A;

/// The section holds no file content, only a size.
pub const SHT_NOBITS: u32 = 8;

/// The section holds a symbol table.
pub const SHT_SYMTAB: u32 = 2;

/// The section holds a string table.
pub const SHT_STRTAB: u32 = 3;

/// One entry of the section header table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Section {
    /// Offset of the section's name in the section name string table.
    pub name: u32,
    /// What the section holds.
    pub kind: u32,
    /// The flags of the section.
    pub flags: u64,
    /// The address the section is loaded at, or zero.
    pub addr: u64,
    /// Offset of the section's content in the file.
    pub offset: u64,
    /// Number of bytes the section holds.
    pub size: u64,
    /// The section this one refers to, by index.
    pub link: u32,
    /// Extra information whose meaning depends on the kind.
    pub info: u32,
    /// The alignment the section claims.
    pub align: u64,
    /// Number of bytes of one entry, for a section of fixed-size entries.
    pub entry_size: u64,
}

/// The section header table of a file, together with the bytes it names.
#[derive(Clone, Copy, Debug)]
pub struct Sections<'a> {
    bytes: &'a [u8],
    offset: usize,
    entry_size: usize,
    count: usize,
    names: usize,
}

impl<'a> Sections<'a> {
    /// The number of sections.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// `true` if the file has no section header table.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The bytes the table was read from.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The section in slot `index`.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<Section> {
        if index >= self.count {
            return None;
        }
        let start = self
            .offset
            .checked_add(index.checked_mul(self.entry_size)?)?;
        let entry = self.bytes.get(start..start.checked_add(SHDR_LEN)?)?;
        let [
            name,
            kind,
            flags_low,
            flags_high,
            addr_low,
            addr_high,
            offset_low,
            offset_high,
            size_low,
            size_high,
            link,
            info,
            align_low,
            align_high,
            entry_low,
            entry_high,
        ] = words::<SHDR_WORDS>(entry);
        Some(Section {
            name,
            kind,
            flags: join(flags_low, flags_high),
            addr: join(addr_low, addr_high),
            offset: join(offset_low, offset_high),
            size: join(size_low, size_high),
            link,
            info,
            align: join(align_low, align_high),
            entry_size: join(entry_low, entry_high),
        })
    }

    /// Every section, in table order.
    pub fn iter(&self) -> impl Iterator<Item = Section> + '_ {
        (0..self.count).filter_map(|index| self.get(index))
    }

    /// The file content of `section`, or `None` if the section holds no
    /// content or reaches beyond the file.
    #[must_use]
    pub fn content(&self, section: Section) -> Option<&'a [u8]> {
        if section.kind == SHT_NOBITS {
            return None;
        }
        let start = usize::try_from(section.offset).ok()?;
        let len = usize::try_from(section.size).ok()?;
        self.bytes.get(start..start.checked_add(len)?)
    }

    /// The name of `section`, read out of the section name string table.
    #[must_use]
    pub fn name_of(&self, section: Section) -> Option<&'a str> {
        let table = self.get(self.names)?;
        string_at(self.content(table)?, section.name)
    }

    /// The first section with the given name.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<Section> {
        self.iter()
            .find(|section| self.name_of(*section) == Some(name))
    }

    /// The content of the first section with the given name.
    #[must_use]
    pub fn content_by_name(&self, name: &str) -> Option<&'a [u8]> {
        self.content(self.by_name(name)?)
    }
}

/// The string at `offset` in a string table, without its terminator.
#[must_use]
pub fn string_at(table: &[u8], offset: u32) -> Option<&str> {
    let start = usize::try_from(offset).ok()?;
    let rest = table.get(start..)?;
    let end = rest.iter().position(|byte| *byte == 0)?;
    core::str::from_utf8(rest.get(..end)?).ok()
}

/// Reads the section header table of `bytes`.
///
/// This validates the file header the way [`crate::image::parse`] does not:
/// it needs the magic, the class, and the byte order, and nothing about
/// segments, so that a file without a loadable segment still yields its
/// sections.
///
/// # Errors
///
/// [`ElfError::TooShort`] if the file is shorter than the file header;
/// [`ElfError::BadMagic`], [`ElfError::NotClass64`], or
/// [`ElfError::NotLittleEndian`] if it is not an `x86-64` object;
/// [`ElfError::SectionHeaderSize`] if an entry is shorter than the
/// structure; [`ElfError::SectionHeaderTable`] if the table reaches beyond
/// the file.
pub fn sections(bytes: &[u8]) -> Result<Sections<'_>, ElfError> {
    let header = bytes.get(..EHDR_LEN).ok_or(ElfError::TooShort)?;
    if header.get(..4) != Some(&[0x7F, b'E', b'L', b'F']) {
        return Err(ElfError::BadMagic);
    }
    if header.get(4) != Some(&2) {
        return Err(ElfError::NotClass64);
    }
    if header.get(5) != Some(&1) {
        return Err(ElfError::NotLittleEndian);
    }
    let offset = read_u64(header, SHOFF_OFFSET).ok_or(ElfError::TooShort)?;
    let entry_size = read_u16(header, SHENTSIZE_OFFSET).ok_or(ElfError::TooShort)?;
    let count = read_u16(header, SHENTSIZE_OFFSET.wrapping_add(2)).ok_or(ElfError::TooShort)?;
    let names = read_u16(header, SHENTSIZE_OFFSET.wrapping_add(4)).ok_or(ElfError::TooShort)?;
    if count == 0 {
        return Ok(Sections {
            bytes,
            offset: 0,
            entry_size: SHDR_LEN,
            count: 0,
            names: 0,
        });
    }
    let entry_size = usize::from(entry_size);
    if entry_size < SHDR_LEN {
        return Err(ElfError::SectionHeaderSize);
    }
    let offset = usize::try_from(offset).map_err(|_| ElfError::SectionHeaderTable)?;
    let count = usize::from(count);
    let end = count
        .checked_mul(entry_size)
        .and_then(|len| offset.checked_add(len))
        .ok_or(ElfError::SectionHeaderTable)?;
    if end > bytes.len() {
        return Err(ElfError::SectionHeaderTable);
    }
    Ok(Sections {
        bytes,
        offset,
        entry_size,
        count,
        names: usize::from(names),
    })
}

/// The two halves of a little-endian `u64`, joined.
fn join(low: u32, high: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

/// The little-endian `u16` at `offset`.
fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([*slice.first()?, *slice.get(1)?]))
}

/// The little-endian `u64` at `offset`.
fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let slice = bytes.get(offset..offset.checked_add(8)?)?;
    let mut value = [0u8; 8];
    for (slot, byte) in value.iter_mut().zip(slice) {
        *slot = *byte;
    }
    Some(u64::from_le_bytes(value))
}
