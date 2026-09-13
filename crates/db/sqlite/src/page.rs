// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One b-tree page: its header, its cell pointers, and the four shapes a
//! cell has.
//!
//! `docs/sqlite/fileformat2.html`, section 1.6, *B-tree Pages*. Page 1
//! carries the database header first, so its b-tree header begins a
//! hundred bytes in; every other page begins with it.

use crate::bytes::{size, u8_at, u16_at, u32_at, varint};
use crate::error::Error;
use crate::header::HEADER_LEN;

/// What a page holds, by the byte it begins with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Type 2: an interior page of an index tree.
    InteriorIndex,
    /// Type 5: an interior page of a table tree.
    InteriorTable,
    /// Type 10: a leaf of an index tree.
    LeafIndex,
    /// Type 13: a leaf of a table tree.
    LeafTable,
}

impl Kind {
    /// The kind a first byte stands for.
    ///
    /// # Errors
    ///
    /// [`Error::PageKind`] for anything but 2, 5, 10 and 13.
    pub const fn from_byte(byte: u8) -> Result<Self, Error> {
        match byte {
            2 => Ok(Kind::InteriorIndex),
            5 => Ok(Kind::InteriorTable),
            10 => Ok(Kind::LeafIndex),
            13 => Ok(Kind::LeafTable),
            other => Err(Error::PageKind(other)),
        }
    }

    /// Whether the page has children, which is what gives it a right-most
    /// pointer and a longer header.
    #[must_use]
    pub const fn is_interior(self) -> bool {
        matches!(self, Kind::InteriorIndex | Kind::InteriorTable)
    }

    /// Whether the page belongs to a table tree, which is what gives its
    /// cells a rowid.
    #[must_use]
    pub const fn is_table(self) -> bool {
        matches!(self, Kind::InteriorTable | Kind::LeafTable)
    }

    /// Bytes of b-tree header: eight, and four more for the right-most
    /// pointer of an interior page.
    #[must_use]
    pub const fn header_len(self) -> usize {
        if self.is_interior() { 12 } else { 8 }
    }
}

/// A payload that may not be whole: what is on the page, how long the
/// whole of it is, and where the rest continues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Payload<'a> {
    /// The bytes of the payload that are on this page.
    pub local: &'a [u8],
    /// How long the payload is in all.
    pub total: usize,
    /// The first overflow page, or nothing when the payload is whole.
    pub overflow: Option<u32>,
}

impl Payload<'_> {
    /// Whether every byte of the payload is on the page it was read from.
    #[must_use]
    pub const fn is_whole(&self) -> bool {
        self.overflow.is_none()
    }
}

/// One cell, in the shape its page gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell<'a> {
    /// A row: its key and its record.
    TableLeaf {
        /// The rowid, which is the key of a table tree.
        rowid: i64,
        /// The record.
        payload: Payload<'a>,
    },
    /// A pointer to a subtree, and the largest rowid in it.
    TableInterior {
        /// The page the subtree begins at.
        child: u32,
        /// The largest key in that subtree.
        rowid: i64,
    },
    /// An index entry.
    IndexLeaf {
        /// The record: the indexed columns and the key they point at.
        payload: Payload<'a>,
    },
    /// A pointer to a subtree, and the entry that separates it from the
    /// next.
    IndexInterior {
        /// The page the subtree begins at.
        child: u32,
        /// The separating entry.
        payload: Payload<'a>,
    },
}

/// A page of a database, read where it lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page<'a> {
    /// The whole page, reserved tail and all.
    bytes: &'a [u8],
    /// Where the b-tree header begins: a hundred on page 1, zero on every
    /// other page.
    start: usize,
    /// Bytes of the page the b-tree layer may use.
    usable: usize,
    /// What the page holds.
    kind: Kind,
}

impl<'a> Page<'a> {
    /// Reads the b-tree header of a page.
    ///
    /// `number` is the page's own number, because page 1 carries the
    /// database header before its b-tree header. `usable` is the page size
    /// less the reserved tail.
    ///
    /// # Errors
    ///
    /// [`Error::PageKind`] for a first byte that is not a page type, and
    /// [`Error::Overrun`] for a page shorter than its own header.
    pub fn parse(bytes: &'a [u8], number: u32, usable: u32) -> Result<Self, Error> {
        let start = if number == 1 { HEADER_LEN } else { 0 };
        let usable = size(u64::from(usable))?;
        if bytes.len() < usable {
            return Err(Error::Overrun);
        }
        let kind = Kind::from_byte(u8_at(bytes, start).ok_or(Error::Overrun)?)?;
        let page = Page {
            bytes,
            start,
            usable,
            kind,
        };
        // The cell pointer array has to fit between the header and the
        // usable end, or the page is not readable at all.
        let pointers = page.cells().checked_mul(2).ok_or(Error::Overrun)?;
        let end = page
            .start
            .checked_add(kind.header_len())
            .and_then(|after| after.checked_add(pointers))
            .ok_or(Error::Overrun)?;
        if end > usable {
            return Err(Error::Overrun);
        }
        Ok(page)
    }

    /// What the page holds.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// How many cells the page has.
    pub fn cells(&self) -> usize {
        u16_at(self.bytes, self.start.saturating_add(3)).map_or(0, usize::from)
    }

    /// The page every key above the last cell lives in, for an interior
    /// page.
    #[must_use]
    pub fn right_most(&self) -> Option<u32> {
        if !self.kind.is_interior() {
            return None;
        }
        u32_at(self.bytes, self.start.saturating_add(8))
    }

    /// Where the cell content area begins. Zero in the header means 65536,
    /// which is the whole of the largest page there is.
    #[must_use]
    pub fn content_start(&self) -> usize {
        match u16_at(self.bytes, self.start.saturating_add(5)) {
            Some(0) | None => 65536,
            Some(offset) => usize::from(offset),
        }
    }

    /// The offset of cell `index`.
    ///
    /// # Errors
    ///
    /// [`Error::Overrun`] for an index the page does not have, or an
    /// offset that lies outside the usable part of the page.
    pub fn cell_offset(&self, index: usize) -> Result<usize, Error> {
        if index >= self.cells() {
            return Err(Error::Overrun);
        }
        let at = self
            .start
            .checked_add(self.kind.header_len())
            .and_then(|base| base.checked_add(index.checked_mul(2)?))
            .ok_or(Error::Overrun)?;
        let offset = usize::from(u16_at(self.bytes, at).ok_or(Error::Overrun)?);
        if offset >= self.usable {
            return Err(Error::Overrun);
        }
        Ok(offset)
    }

    /// Reads cell `index`.
    ///
    /// # Errors
    ///
    /// [`Error::Overrun`] for a cell that reaches past the page, and
    /// [`Error::Varint`] for a length or a key that does not end inside
    /// it.
    pub fn cell(&self, index: usize) -> Result<Cell<'a>, Error> {
        let offset = self.cell_offset(index)?;
        let cell = self.bytes.get(offset..self.usable).ok_or(Error::Overrun)?;
        match self.kind {
            Kind::InteriorTable => {
                let child = child_page(u32_at(cell, 0).ok_or(Error::Overrun)?)?;
                let (key, _) = varint(cell.get(4..).ok_or(Error::Overrun)?)?;
                Ok(Cell::TableInterior {
                    child,
                    rowid: crate::bytes::signed(key),
                })
            }
            Kind::LeafTable => {
                let (total, read) = varint(cell)?;
                let rest = cell.get(read..).ok_or(Error::Overrun)?;
                let (key, key_len) = varint(rest)?;
                let after = rest.get(key_len..).ok_or(Error::Overrun)?;
                Ok(Cell::TableLeaf {
                    rowid: crate::bytes::signed(key),
                    payload: self.payload(after, size(total)?)?,
                })
            }
            Kind::LeafIndex => {
                let (total, read) = varint(cell)?;
                let after = cell.get(read..).ok_or(Error::Overrun)?;
                Ok(Cell::IndexLeaf {
                    payload: self.payload(after, size(total)?)?,
                })
            }
            Kind::InteriorIndex => {
                let child = child_page(u32_at(cell, 0).ok_or(Error::Overrun)?)?;
                let rest = cell.get(4..).ok_or(Error::Overrun)?;
                let (total, read) = varint(rest)?;
                let after = rest.get(read..).ok_or(Error::Overrun)?;
                Ok(Cell::IndexInterior {
                    child,
                    payload: self.payload(after, size(total)?)?,
                })
            }
        }
    }

    /// Every cell of the page, in the order the pointer array names them.
    pub fn cells_iter(&self) -> impl Iterator<Item = Result<Cell<'a>, Error>> + '_ {
        (0..self.cells()).map(move |index| self.cell(index))
    }

    /// Splits a payload into what is on the page and what follows it.
    fn payload(&self, after_header: &'a [u8], total: usize) -> Result<Payload<'a>, Error> {
        let local_len = local_len(total, self.usable, self.kind);
        let local = after_header.get(..local_len).ok_or(Error::Overrun)?;
        if local_len == total {
            return Ok(Payload {
                local,
                total,
                overflow: None,
            });
        }
        let page = u32_at(after_header, local_len).ok_or(Error::Overrun)?;
        if page == 0 {
            return Err(Error::Overflow(0));
        }
        Ok(Payload {
            local,
            total,
            overflow: Some(page),
        })
    }
}

/// A child pointer, which is a page number and therefore never zero.
const fn child_page(number: u32) -> Result<u32, Error> {
    if number == 0 {
        return Err(Error::Page(0));
    }
    Ok(number)
}

/// How many bytes of a payload of `total` bytes stay on the page.
///
/// Section 1.6, *Cell Payload Overflow Pages*, with U the usable size, P
/// the payload, X the largest payload that stays whole, and M the smallest
/// that is ever left on a page. The remainder rule exists so that the last
/// overflow page is as full as it can be.
fn local_len(total: usize, usable: usize, kind: Kind) -> usize {
    let x = if kind.is_table() {
        usable.saturating_sub(35)
    } else {
        usable
            .saturating_sub(12)
            .saturating_mul(64)
            .saturating_div(255)
            .saturating_sub(23)
    };
    if total <= x {
        return total;
    }
    let m = usable
        .saturating_sub(12)
        .saturating_mul(32)
        .saturating_div(255)
        .saturating_sub(23);
    let span = usable.saturating_sub(4).max(1);
    let k = m.saturating_add(total.saturating_sub(m).checked_rem(span).unwrap_or(0));
    if k <= x { k } else { m }
}
