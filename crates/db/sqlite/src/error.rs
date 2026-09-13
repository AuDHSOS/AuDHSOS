// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a read was refused.

use core::fmt;

/// What a call of this crate refused, and why. Every one of them names a
/// rule of the file format rather than a symptom, so a refusal says which
/// sentence of the format the file breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// The first sixteen bytes are not `SQLite format 3\0`.
    Magic,
    /// The file is shorter than the hundred-byte header.
    Truncated,
    /// The page size is not a power of two between 512 and 65536.
    PageSize(u32),
    /// The reserved space at the end of a page leaves fewer than 480
    /// usable bytes, which the format forbids.
    Reserved(u8),
    /// The payload fractions are not 64, 32 and 32.
    Fractions,
    /// The text encoding is not 1, 2 or 3.
    Encoding(u32),
    /// A page number of zero, or one the file does not hold.
    Page(u32),
    /// A page whose first byte is not 2, 5, 10 or 13.
    PageKind(u8),
    /// A cell pointer, a cell, or a record that reaches past its page.
    Overrun,
    /// A varint that does not end inside the bytes it was read from.
    Varint,
    /// A serial type of 10 or 11, which the format reserves.
    SerialType(u64),
    /// A b-tree deeper than this crate walks.
    Depth,
    /// An overflow chain that reaches a page it has already read, or that
    /// ends before the payload is whole.
    Overflow(u32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Magic => f.write_str("not a SQLite database: the header string is wrong"),
            Error::Truncated => f.write_str("shorter than the hundred-byte header"),
            Error::PageSize(size) => {
                write!(f, "page size {size} is not a power of two in 512..=65536")
            }
            Error::Reserved(bytes) => write!(f, "{bytes} reserved bytes leave a page too small"),
            Error::Fractions => f.write_str("the payload fractions are not 64, 32 and 32"),
            Error::Encoding(code) => {
                write!(f, "text encoding {code} is not UTF-8, UTF-16le or UTF-16be")
            }
            Error::Page(number) => write!(f, "page {number} is not in the file"),
            Error::PageKind(byte) => write!(f, "{byte} is not a b-tree page type"),
            Error::Overrun => f.write_str("a cell or a record reaches past its page"),
            Error::Varint => f.write_str("a varint runs past the bytes it was read from"),
            Error::SerialType(code) => write!(f, "serial type {code} is reserved"),
            Error::Depth => f.write_str("the b-tree is deeper than this crate walks"),
            Error::Overflow(page) => write!(f, "the overflow chain at page {page} is broken"),
        }
    }
}
