// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why an ELF image was rejected.

use core::fmt;

/// Why an ELF image was rejected. The header is checked field by field in
/// the order in which the fields appear, then the program header table,
/// then every segment, then the checks that span segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElfError {
    /// Fewer bytes than the file header needs.
    TooShort,
    /// The first four bytes are not `7F 45 4C 46`.
    BadMagic,
    /// The class byte does not say 64-bit.
    NotClass64,
    /// The data byte does not say little-endian.
    NotLittleEndian,
    /// The type is not `ET_EXEC`.
    NotExecutable,
    /// The machine is not `x86-64`.
    WrongMachine,
    /// The file header is shorter than the structure.
    HeaderSize,
    /// A program header entry is shorter than the structure.
    ProgramHeaderSize,
    /// The program header table reaches beyond the file.
    ProgramHeaderTable,
    /// The file holds no loadable segment.
    NoSegments,
    /// The file holds more loadable segments than the fixed capacity.
    TooManySegments,
    /// A segment's file range reaches beyond the file.
    SegmentFileRange {
        /// Index of the segment in the program header table.
        index: usize,
    },
    /// A segment holds less memory than file content.
    SegmentMemorySize {
        /// Index of the segment in the program header table.
        index: usize,
    },
    /// A segment's alignment is not a power of two, or file offset and
    /// virtual address disagree modulo the alignment.
    SegmentAlignment {
        /// Index of the segment in the program header table.
        index: usize,
    },
    /// A segment lies outside the bounds the caller allows.
    SegmentOutsideBounds {
        /// Index of the segment in the program header table.
        index: usize,
    },
    /// Two segments share memory.
    SegmentsOverlap,
    /// A segment is both writable and executable.
    WritableAndExecutable {
        /// Index of the segment in the program header table.
        index: usize,
    },
    /// The entry point lies in no executable segment.
    EntryNotExecutable,
    /// An offset or a length leaves the representable range.
    Overflow,
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ElfError::TooShort => f.write_str("the file is shorter than the ELF header"),
            ElfError::BadMagic => f.write_str("the ELF magic does not match"),
            ElfError::NotClass64 => f.write_str("the file is not a 64-bit ELF file"),
            ElfError::NotLittleEndian => f.write_str("the file is not little-endian"),
            ElfError::NotExecutable => f.write_str("the file is not an executable"),
            ElfError::WrongMachine => f.write_str("the file is not for x86-64"),
            ElfError::HeaderSize => f.write_str("the ELF header is too short"),
            ElfError::ProgramHeaderSize => f.write_str("a program header entry is too short"),
            ElfError::ProgramHeaderTable => {
                f.write_str("the program header table reaches beyond the file")
            }
            ElfError::NoSegments => f.write_str("the file holds no loadable segment"),
            ElfError::TooManySegments => f.write_str("the file holds too many loadable segments"),
            ElfError::SegmentFileRange { index } => {
                write!(f, "segment {index} reaches beyond the file")
            }
            ElfError::SegmentMemorySize { index } => {
                write!(f, "segment {index} holds less memory than file content")
            }
            ElfError::SegmentAlignment { index } => {
                write!(f, "segment {index} is not aligned as it claims")
            }
            ElfError::SegmentOutsideBounds { index } => {
                write!(f, "segment {index} lies outside the allowed range")
            }
            ElfError::SegmentsOverlap => f.write_str("two segments share memory"),
            ElfError::WritableAndExecutable { index } => {
                write!(f, "segment {index} is both writable and executable")
            }
            ElfError::EntryNotExecutable => {
                f.write_str("the entry point lies in no executable segment")
            }
            ElfError::Overflow => {
                f.write_str("an offset or a length leaves the representable range")
            }
        }
    }
}
