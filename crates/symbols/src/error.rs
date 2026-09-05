// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a file could not be read for symbols.

use core::fmt;

use audhsos_elf::ElfError;

/// Why a file could not be read for symbols.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolError {
    /// The file is not an ELF object this crate can read.
    Elf(ElfError),
    /// A structure reaches beyond the bytes that hold it.
    Truncated,
    /// A number does not fit the width the format allows for it.
    Overflow,
    /// The line program is of a version this crate does not read.
    LineVersion(u16),
    /// The line program divides by a range of zero.
    LineRange,
    /// An entry of the file table uses a form this crate does not read.
    UnknownForm(u64),
}

impl fmt::Display for SymbolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SymbolError::Elf(error) => write!(f, "{error}"),
            SymbolError::Truncated => f.write_str("a structure reaches beyond its bytes"),
            SymbolError::Overflow => f.write_str("a number is too large for its field"),
            SymbolError::LineVersion(version) => {
                write!(f, "the line program is version {version}, not 4 or 5")
            }
            SymbolError::LineRange => f.write_str("the line program has a line range of zero"),
            SymbolError::UnknownForm(form) => write!(f, "unknown form {form:#x} in a file table"),
        }
    }
}

impl From<ElfError> for SymbolError {
    fn from(error: ElfError) -> Self {
        SymbolError::Elf(error)
    }
}
