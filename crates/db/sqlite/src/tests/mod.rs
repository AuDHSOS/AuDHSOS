// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.
//!
//! The fixtures under `fixtures/` were written by the `sqlite3` shell
//! itself, which makes them the format as it is and not as this crate
//! reads it. What produced each is named where it is used.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

mod bytes;
mod error;
mod header;
mod image;
mod matrix;
mod page;
mod record;
mod token;

/// One table, three rows, every storage class, page size 4096, UTF-8.
///
/// `CREATE TABLE t(a INTEGER, b TEXT, c REAL, d BLOB); INSERT INTO t
/// VALUES (1,'one',1.5,x'0102'), (2,'two',-2.5,NULL), (-3,'',0.0,x'ff');`
pub(super) const SMALL: &[u8] = include_bytes!("fixtures/small.db");

/// Four hundred rows over 512-byte pages, which is small enough that the
/// tree has an interior page and the walk has to descend.
///
/// `PRAGMA page_size=512; CREATE TABLE wide(n INTEGER, s TEXT);` filled
/// with `'row ' || i` for `i` in 1..=400.
pub(super) const PAGE512: &[u8] = include_bytes!("fixtures/page512.db");

/// Text in UTF-16, little-endian.
///
/// `PRAGMA encoding='UTF-16le'; CREATE TABLE u(t TEXT); INSERT INTO u
/// VALUES ('abc'), ('äöü');`
pub(super) const UTF16: &[u8] = include_bytes!("fixtures/utf16.db");

/// One row whose text is eighteen thousand bytes, which no page holds.
///
/// `CREATE TABLE big(t TEXT); INSERT INTO big VALUES
/// (replace(hex(zeroblob(9000)),'0','x'));`
pub(super) const OVERFLOW: &[u8] = include_bytes!("fixtures/overflow.db");

/// A table with two indexes over it, so that index pages exist to read.
///
/// `CREATE TABLE k(a INTEGER, b TEXT); CREATE INDEX ka ON k(a); CREATE
/// UNIQUE INDEX kb ON k(b);` filled with a hundred rows.
pub(super) const INDEXED: &[u8] = include_bytes!("fixtures/indexed.db");

/// A varint, written the way section 1.6 describes, so that the reader is
/// tested against something other than itself.
pub(super) fn varint(value: u64) -> ([u8; 9], usize) {
    let mut out = [0u8; 9];
    let byte = |bits: u64| u8::try_from(bits & 0xff).unwrap_or(0);
    if value > 0x00ff_ffff_ffff_ffff {
        // Nine bytes: fifty-six bits in sevens, then a whole byte.
        let high = value >> 8;
        for (at, slot) in out.iter_mut().take(8).enumerate() {
            *slot = byte((high >> (7 * (7 - at))) & 0x7f) | 0x80;
        }
        out[8] = byte(value);
        return (out, 9);
    }
    let mut groups = [0u8; 8];
    let mut count = 0;
    let mut rest = value;
    loop {
        groups[count] = byte(rest & 0x7f);
        rest >>= 7;
        count += 1;
        if rest == 0 {
            break;
        }
    }
    for (at, slot) in out.iter_mut().take(count).enumerate() {
        let group = groups[count - 1 - at];
        *slot = if at + 1 == count { group } else { group | 0x80 };
    }
    (out, count)
}

/// A database laid out by hand: a header, a page 1 that is an empty leaf,
/// and whatever pages a test appends. It is how a file the shell cannot
/// write — a broken overflow chain, a tree that points into itself — is
/// put in front of the reader.
pub(super) struct Builder {
    /// Bytes per page.
    page_size: usize,
    /// The file so far.
    bytes: Vec<u8>,
}

impl Builder {
    /// A database of one page: the header and an empty leaf table.
    pub(super) fn new(page_size: usize) -> Self {
        let mut bytes = SMALL[..100].to_vec();
        let field = if page_size == 65536 {
            1
        } else {
            u16::try_from(page_size).unwrap()
        };
        bytes[16..18].copy_from_slice(&field.to_be_bytes());
        bytes.resize(page_size, 0);
        // Page 1 after the header: a leaf table page with no cells.
        bytes[100] = 13;
        bytes[105..107].copy_from_slice(&u16::try_from(page_size).unwrap_or(0).to_be_bytes());
        Builder { page_size, bytes }
    }

    /// Appends one page, padded to the page size.
    pub(super) fn page(mut self, page: &[u8]) -> Self {
        let start = self.bytes.len();
        self.bytes.resize(start + self.page_size, 0);
        self.bytes[start..start + page.len()].copy_from_slice(page);
        self
    }

    /// The file, with the header's page count matching its length.
    pub(super) fn finish(mut self) -> Vec<u8> {
        let pages = u32::try_from(self.bytes.len() / self.page_size).unwrap();
        self.bytes[28..32].copy_from_slice(&pages.to_be_bytes());
        self.bytes
    }
}

/// A leaf table page holding one cell: a payload of `total` bytes of which
/// `local` are on the page, and the overflow page that follows.
pub(super) fn leaf_with_overflow(page_size: usize, total: u64, local: usize, next: u32) -> Vec<u8> {
    let (size_bytes, size_len) = varint(total);
    let mut cell = Vec::new();
    cell.extend_from_slice(&size_bytes[..size_len]);
    cell.push(1); // the rowid, as a one-byte varint
    cell.extend(core::iter::repeat_n(b'a', local));
    cell.extend_from_slice(&next.to_be_bytes());
    let offset = page_size - cell.len();
    let mut page = vec![0u8; page_size];
    page[0] = 13;
    page[3..5].copy_from_slice(&1u16.to_be_bytes());
    page[5..7].copy_from_slice(&u16::try_from(offset).unwrap().to_be_bytes());
    page[8..10].copy_from_slice(&u16::try_from(offset).unwrap().to_be_bytes());
    page[offset..].copy_from_slice(&cell);
    page
}

/// An interior table page with no cells whose right-most pointer is
/// `child`, which is how a tree of any depth is built.
pub(super) fn interior_to(page_size: usize, child: u32) -> Vec<u8> {
    let mut page = vec![0u8; page_size];
    page[0] = 5;
    page[5..7].copy_from_slice(&u16::try_from(page_size).unwrap_or(0).to_be_bytes());
    page[8..12].copy_from_slice(&child.to_be_bytes());
    page
}
