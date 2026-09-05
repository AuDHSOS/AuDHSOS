// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The ELF64 file header, the program header table, and the validated
//! image they describe.
//!
//! Invariants: every segment of an [`Image`] lies inside the file, inside
//! the caller's bounds, and is never both writable and executable; no two
//! segments share memory; the segments are sorted by virtual address; the
//! entry point lies in an executable segment.

use crate::error::ElfError;

/// Length of the ELF64 file header in bytes.
pub const EHDR_LEN: usize = 64;

/// Length of one ELF64 program header entry in bytes.
pub const PHDR_LEN: usize = 56;

/// The largest number of loadable segments an image may have.
pub const MAX_SEGMENTS: usize = 16;

/// The first four bytes of every ELF file, read as a little-endian `u32`.
const MAGIC: u32 = 0x464C_457F;

/// `ELFCLASS64`.
const CLASS_64: u32 = 2;

/// `ELFDATA2LSB`.
const DATA_LSB: u32 = 1;

/// `ET_EXEC`.
const TYPE_EXEC: u32 = 2;

/// `EM_X86_64`.
const MACHINE_X86_64: u32 = 0x3E;

/// `PT_LOAD`.
const PT_LOAD: u32 = 1;

/// `PF_X`.
const PF_X: u32 = 1;

/// `PF_W`.
const PF_W: u32 = 2;

/// Number of `u32` words in the file header.
const EHDR_WORDS: usize = EHDR_LEN / 4;

/// Number of `u32` words in one program header entry.
const PHDR_WORDS: usize = PHDR_LEN / 4;

/// One loadable segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Segment {
    /// Offset of the segment's content in the file.
    pub file_offset: u64,
    /// Number of bytes the file holds for the segment.
    pub file_size: u64,
    /// Virtual address the segment is loaded at.
    pub vaddr: u64,
    /// Number of bytes the segment occupies in memory.
    pub mem_size: u64,
    /// Alignment the segment claims; zero or a power of two.
    pub align: u64,
    /// Writing is allowed.
    pub write: bool,
    /// Executing is allowed.
    pub execute: bool,
}

impl Segment {
    /// The first address above the segment, if it is representable.
    #[must_use]
    pub const fn end(self) -> Option<u64> {
        self.vaddr.checked_add(self.mem_size)
    }

    /// `true` if `address` lies in the segment.
    #[must_use]
    pub const fn contains(self, address: u64) -> bool {
        match self.end() {
            Some(end) => address >= self.vaddr && address < end,
            None => false,
        }
    }

    /// `true` if the segments share at least one byte of memory.
    #[must_use]
    pub const fn overlaps(self, other: Segment) -> bool {
        match (self.end(), other.end()) {
            (Some(end), Some(other_end)) => {
                self.mem_size != 0
                    && other.mem_size != 0
                    && self.vaddr < other_end
                    && other.vaddr < end
            }
            _ => false,
        }
    }
}

/// The bounds a caller allows the segments to lie in, both inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Constraints {
    /// Lowest address a segment may start at.
    pub lowest_vaddr: u64,
    /// Highest address a segment may reach.
    pub highest_vaddr: u64,
}

impl Constraints {
    /// `true` if `start .. start + len` lies inside the bounds. An empty
    /// range only has to start inside them.
    #[must_use]
    pub const fn allows(self, start: u64, len: u64) -> bool {
        if start < self.lowest_vaddr || start > self.highest_vaddr {
            return false;
        }
        if len == 0 {
            return true;
        }
        match start.checked_add(len.wrapping_sub(1)) {
            Some(last) => last <= self.highest_vaddr,
            None => false,
        }
    }
}

/// A validated executable together with the bytes it was parsed from.
#[derive(Clone, Copy, Debug)]
pub struct Image<'a> {
    /// The bytes the image was parsed from.
    pub bytes: &'a [u8],
    /// The entry point.
    pub entry: u64,
    segments: [Option<Segment>; MAX_SEGMENTS],
    count: usize,
}

impl<'a> Image<'a> {
    /// The loadable segments, lowest virtual address first.
    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        self.segments.iter().take(self.count).flatten().copied()
    }

    /// The number of loadable segments.
    #[must_use]
    pub const fn segment_count(&self) -> usize {
        self.count
    }

    /// The file content of `segment`. A segment that does not belong to
    /// this image yields an empty slice.
    #[must_use]
    pub fn segment_bytes(&self, segment: Segment) -> &'a [u8] {
        let start = usize::try_from(segment.file_offset).unwrap_or(usize::MAX);
        let len = usize::try_from(segment.file_size).unwrap_or(usize::MAX);
        start
            .checked_add(len)
            .and_then(|end| self.bytes.get(start..end))
            .unwrap_or(&[])
    }

    /// The highest address any segment reaches, if the image has segments.
    #[must_use]
    pub fn highest_address(&self) -> Option<u64> {
        self.segments()
            .filter_map(Segment::end)
            .max()
            .map(|end| end.saturating_sub(1))
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

/// The low half of a `u32` word.
const fn low(word: u32) -> u32 {
    word & 0xFFFF
}

/// The high half of a `u32` word.
const fn high(word: u32) -> u32 {
    word >> 16
}

/// The header fields the parser uses.
struct FileHeader {
    entry: u64,
    phoff: u64,
    phentsize: u32,
    phnum: u32,
}

fn parse_header(bytes: &[u8]) -> Result<FileHeader, ElfError> {
    let header = bytes.get(..EHDR_LEN).ok_or(ElfError::TooShort)?;
    let w = words::<EHDR_WORDS>(header);
    let [
        magic,
        ident,
        _ident2,
        _ident3,
        kind,
        _version,
        entry_low,
        entry_high,
        phoff_low,
        phoff_high,
        _shoff_low,
        _shoff_high,
        _flags,
        sizes,
        counts,
        _indices,
    ] = w;
    if magic != MAGIC {
        return Err(ElfError::BadMagic);
    }
    if ident & 0xFF != CLASS_64 {
        return Err(ElfError::NotClass64);
    }
    if (ident >> 8) & 0xFF != DATA_LSB {
        return Err(ElfError::NotLittleEndian);
    }
    if low(kind) != TYPE_EXEC {
        return Err(ElfError::NotExecutable);
    }
    if high(kind) != MACHINE_X86_64 {
        return Err(ElfError::WrongMachine);
    }
    let ehsize = low(sizes);
    if usize::try_from(ehsize).unwrap_or(0) < EHDR_LEN {
        return Err(ElfError::HeaderSize);
    }
    let phentsize = high(sizes);
    if usize::try_from(phentsize).unwrap_or(0) < PHDR_LEN {
        return Err(ElfError::ProgramHeaderSize);
    }
    Ok(FileHeader {
        entry: join(entry_low, entry_high),
        phoff: join(phoff_low, phoff_high),
        phentsize,
        phnum: low(counts),
    })
}

/// Parses and validates an ELF64 executable for `x86_64`.
///
/// # Errors
///
/// One [`ElfError`] per rejected field; the header is checked field by
/// field, then the program header table, then every segment, then the
/// checks that span segments.
pub fn parse(bytes: &[u8], constraints: Constraints) -> Result<Image<'_>, ElfError> {
    let header = parse_header(bytes)?;
    let file_len = u64::try_from(bytes.len()).map_err(|_| ElfError::Overflow)?;
    let table_len = u64::from(header.phentsize)
        .checked_mul(u64::from(header.phnum))
        .ok_or(ElfError::Overflow)?;
    let table_end = header
        .phoff
        .checked_add(table_len)
        .ok_or(ElfError::Overflow)?;
    if table_end > file_len {
        return Err(ElfError::ProgramHeaderTable);
    }

    let mut segments: [Option<Segment>; MAX_SEGMENTS] = [None; MAX_SEGMENTS];
    let mut count = 0usize;
    for index in 0..usize::try_from(header.phnum).unwrap_or(0) {
        let offset = usize::try_from(header.phoff)
            .ok()
            .and_then(|base| {
                base.checked_add(index.checked_mul(usize::try_from(header.phentsize).ok()?)?)
            })
            .ok_or(ElfError::Overflow)?;
        let entry = bytes
            .get(offset..offset.saturating_add(PHDR_LEN))
            .ok_or(ElfError::ProgramHeaderTable)?;
        let [
            kind,
            flags,
            off_low,
            off_high,
            vaddr_low,
            vaddr_high,
            _paddr_low,
            _paddr_high,
            filesz_low,
            filesz_high,
            memsz_low,
            memsz_high,
            align_low,
            align_high,
        ] = words::<PHDR_WORDS>(entry);
        if kind != PT_LOAD {
            continue;
        }
        let segment = Segment {
            file_offset: join(off_low, off_high),
            file_size: join(filesz_low, filesz_high),
            vaddr: join(vaddr_low, vaddr_high),
            mem_size: join(memsz_low, memsz_high),
            align: join(align_low, align_high),
            write: flags & PF_W != 0,
            execute: flags & PF_X != 0,
        };
        check_segment(&segment, index, file_len, constraints)?;
        let slot = segments.get_mut(count).ok_or(ElfError::TooManySegments)?;
        *slot = Some(segment);
        count = count.saturating_add(1);
    }
    if count == 0 {
        return Err(ElfError::NoSegments);
    }
    sort_by_vaddr(&mut segments, count);
    check_overlaps(&segments, count)?;
    if !segments
        .iter()
        .take(count)
        .flatten()
        .any(|segment| segment.execute && segment.contains(header.entry))
    {
        return Err(ElfError::EntryNotExecutable);
    }
    Ok(Image {
        bytes,
        entry: header.entry,
        segments,
        count,
    })
}

fn check_segment(
    segment: &Segment,
    index: usize,
    file_len: u64,
    constraints: Constraints,
) -> Result<(), ElfError> {
    if segment.write && segment.execute {
        return Err(ElfError::WritableAndExecutable { index });
    }
    if segment.file_size > segment.mem_size {
        return Err(ElfError::SegmentMemorySize { index });
    }
    let file_end = segment
        .file_offset
        .checked_add(segment.file_size)
        .ok_or(ElfError::Overflow)?;
    if file_end > file_len {
        return Err(ElfError::SegmentFileRange { index });
    }
    if segment.align != 0 {
        if !segment.align.is_power_of_two() {
            return Err(ElfError::SegmentAlignment { index });
        }
        let mask = segment.align.wrapping_sub(1);
        if segment.vaddr & mask != segment.file_offset & mask {
            return Err(ElfError::SegmentAlignment { index });
        }
    }
    if !constraints.allows(segment.vaddr, segment.mem_size) {
        return Err(ElfError::SegmentOutsideBounds { index });
    }
    Ok(())
}

/// Insertion sort by virtual address; the list holds at most
/// [`MAX_SEGMENTS`] entries.
fn sort_by_vaddr(segments: &mut [Option<Segment>; MAX_SEGMENTS], count: usize) {
    let mut index = 1;
    while index < count {
        let mut position = index;
        while position > 0 {
            let previous = position.saturating_sub(1);
            match (
                segments.get(previous).copied().flatten(),
                segments.get(position).copied().flatten(),
            ) {
                (Some(left), Some(right)) if left.vaddr > right.vaddr => {
                    if let Some(slot) = segments.get_mut(previous) {
                        *slot = Some(right);
                    }
                    if let Some(slot) = segments.get_mut(position) {
                        *slot = Some(left);
                    }
                }
                _ => break,
            }
            position = previous;
        }
        index = index.saturating_add(1);
    }
}

fn check_overlaps(
    segments: &[Option<Segment>; MAX_SEGMENTS],
    count: usize,
) -> Result<(), ElfError> {
    let mut index = 1;
    while index < count {
        let previous = index.saturating_sub(1);
        if let (Some(left), Some(right)) = (
            segments.get(previous).copied().flatten(),
            segments.get(index).copied().flatten(),
        ) && left.overlaps(right)
        {
            return Err(ElfError::SegmentsOverlap);
        }
        index = index.saturating_add(1);
    }
    Ok(())
}
