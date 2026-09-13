// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One b-tree page: its header, its cell pointers, and the four shapes a
//! cell has.
//!
//! `docs/sqlite/fileformat2.html`, section 1.6, *B-tree Pages*. Page 1
//! carries the database header first, so its b-tree header begins a
//! hundred bytes in; every other page begins with it.
//!
//! A page is written as well as read: [`build`] lays the cells out the
//! way `rebuildPage` in `src/btree.c` lays them out, which is the shape
//! a page filled in key order has.

use alloc::vec::Vec;

use crate::bytes::{put_varint, size, u8_at, u16_at, u32_at, varint};
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

    /// The byte a page of this kind begins with.
    #[must_use]
    pub const fn byte(self) -> u8 {
        match self {
            Kind::InteriorIndex => 2,
            Kind::InteriorTable => 5,
            Kind::LeafIndex => 10,
            Kind::LeafTable => 13,
        }
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
        let usable = size(u64::from(usable));
        if bytes.len() < usable {
            return Err(Error::Overrun);
        }
        // A page is at least 480 bytes, so the byte the type is in is
        // there whether the header lies before it or not.
        let kind = Kind::from_byte(u8_at(bytes, start).unwrap_or(0))?;
        let page = Page {
            bytes,
            start,
            usable,
            kind,
        };
        // The cell pointer array has to fit between the header and the
        // usable end, or the page is not readable at all.
        let pointers = page.cells().saturating_mul(2);
        let end = page
            .start
            .saturating_add(kind.header_len())
            .saturating_add(pointers);
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
            .saturating_add(self.kind.header_len())
            .saturating_add(index.saturating_mul(2));
        // The pointer array was checked to lie inside the page, so the
        // two bytes are there; a pointer that leaves the page is what the
        // test below it refuses.
        let offset = usize::from(u16_at(self.bytes, at).unwrap_or(u16::MAX));
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
        // The offset lies inside the usable part of the page, which is
        // what `cell_offset` has just checked.
        let cell = self.bytes.get(offset..self.usable).unwrap_or_default();
        match self.kind {
            Kind::InteriorTable => {
                let child = child_page(u32_at(cell, 0).ok_or(Error::Overrun)?)?;
                // The child pointer was read out of these bytes, so the
                // key after it is inside them.
                let (key, _) = varint(cell.get(4..).unwrap_or_default())?;
                Ok(Cell::TableInterior {
                    child,
                    rowid: crate::bytes::signed(key),
                })
            }
            Kind::LeafTable => {
                let (total, read) = varint(cell)?;
                // A varint was read out of these bytes, so what follows
                // it is inside them.
                let rest = cell.get(read..).unwrap_or_default();
                let (key, key_len) = varint(rest)?;
                let after = rest.get(key_len..).unwrap_or_default();
                Ok(Cell::TableLeaf {
                    rowid: crate::bytes::signed(key),
                    payload: self.payload(after, size(total))?,
                })
            }
            Kind::LeafIndex => {
                let (total, read) = varint(cell)?;
                let after = cell.get(read..).unwrap_or_default();
                Ok(Cell::IndexLeaf {
                    payload: self.payload(after, size(total))?,
                })
            }
            Kind::InteriorIndex => {
                let child = child_page(u32_at(cell, 0).ok_or(Error::Overrun)?)?;
                let rest = cell.get(4..).unwrap_or_default();
                let (total, read) = varint(rest)?;
                let after = rest.get(read..).unwrap_or_default();
                Ok(Cell::IndexInterior {
                    child,
                    payload: self.payload(after, size(total))?,
                })
            }
        }
    }

    /// The row of cell `index`: its key and its record.
    ///
    /// # Errors
    ///
    /// [`Error::PageKind`] where this is not a table leaf, and the errors
    /// of [`Page::cell`].
    pub fn row(&self, index: usize) -> Result<(i64, Payload<'a>), Error> {
        match self.cell(index)? {
            Cell::TableLeaf { rowid, payload } => Ok((rowid, payload)),
            Cell::TableInterior { .. } | Cell::IndexLeaf { .. } | Cell::IndexInterior { .. } => {
                Err(Error::PageKind(self.kind.byte()))
            }
        }
    }

    /// The entry of cell `index`: the record an index page carries.
    ///
    /// # Errors
    ///
    /// [`Error::PageKind`] where this is a table page, and the errors of
    /// [`Page::cell`].
    pub fn entry(&self, index: usize) -> Result<Payload<'a>, Error> {
        match self.cell(index)? {
            Cell::IndexLeaf { payload } | Cell::IndexInterior { payload, .. } => Ok(payload),
            Cell::TableLeaf { .. } | Cell::TableInterior { .. } => {
                Err(Error::PageKind(self.kind.byte()))
            }
        }
    }

    /// The page cell `index` points at, for an interior page.
    ///
    /// # Errors
    ///
    /// [`Error::PageKind`] where this is a leaf, and the errors of
    /// [`Page::cell`].
    pub fn child(&self, index: usize) -> Result<u32, Error> {
        match self.cell(index)? {
            Cell::TableInterior { child, .. } | Cell::IndexInterior { child, .. } => Ok(child),
            Cell::TableLeaf { .. } | Cell::IndexLeaf { .. } => {
                Err(Error::PageKind(self.kind.byte()))
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

/// One cell as the bytes a page holds it in: the child pointer of an
/// interior page, the length of the payload, the key of a table tree,
/// and what of the payload is on the page, with the first overflow page
/// after it where the payload runs on.
#[must_use]
pub fn write_cell(cell: &Cell<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    match cell {
        Cell::TableInterior { child, rowid } => {
            out.extend_from_slice(&child.to_be_bytes());
            put_varint(&mut out, rowid.cast_unsigned());
        }
        Cell::TableLeaf { rowid, payload } => {
            put_varint(&mut out, whole(payload.total));
            put_varint(&mut out, rowid.cast_unsigned());
            put_payload(&mut out, payload);
        }
        Cell::IndexLeaf { payload } => {
            put_varint(&mut out, whole(payload.total));
            put_payload(&mut out, payload);
        }
        Cell::IndexInterior { child, payload } => {
            out.extend_from_slice(&child.to_be_bytes());
            put_varint(&mut out, whole(payload.total));
            put_payload(&mut out, payload);
        }
    }
    out
}

/// What of a payload the cell holds, and the page the rest is on.
fn put_payload(out: &mut Vec<u8>, payload: &Payload<'_>) {
    out.extend_from_slice(payload.local);
    if let Some(page) = payload.overflow {
        out.extend_from_slice(&page.to_be_bytes());
    }
}

/// A length as the number a varint is written from.
fn whole(total: usize) -> u64 {
    u64::try_from(total).unwrap_or(u64::MAX)
}

/// A page holding `cells`, in the order they are given.
///
/// This is `rebuildPage`: the pointer array names the cells in that
/// order, and the content of each is taken from the end of the usable
/// part downward, so the first cell lies highest. The page carries no
/// freeblock and no fragmented byte, which is what a page built in one
/// pass has. `number` is the page's own number, because page 1 carries
/// the database header before its b-tree header, and those hundred bytes
/// are left as they are.
///
/// Answers nothing where the cells and their pointers do not fit.
#[must_use]
pub fn build(
    kind: Kind,
    number: u32,
    page_size: usize,
    usable: usize,
    cells: &[Vec<u8>],
    right_most: Option<u32>,
) -> Option<Vec<u8>> {
    let start = if number == 1 { HEADER_LEN } else { 0 };
    if usable > page_size || start.saturating_add(kind.header_len()) > usable {
        return None;
    }
    let mut out = alloc::vec![0u8; page_size];
    let mut content = usable;
    let mut pointers = Vec::with_capacity(cells.len());
    for cell in cells {
        content = content.checked_sub(cell.len())?;
        for (slot, byte) in out.iter_mut().skip(content).zip(cell) {
            *slot = *byte;
        }
        pointers.push(content);
    }
    let array = start.saturating_add(kind.header_len());
    if array.saturating_add(cells.len().saturating_mul(2)) > content {
        return None;
    }
    for (slot, byte) in out.iter_mut().skip(start).zip([kind.byte()]) {
        *slot = byte;
    }
    // The first freeblock and the fragmented bytes are nought, which the
    // page already is.
    put16(
        &mut out,
        start.saturating_add(3),
        u16::try_from(cells.len()).ok()?,
    );
    // A content area that begins at 65536 is written as nought, which is
    // the whole of the largest page there is.
    put16(
        &mut out,
        start.saturating_add(5),
        u16::try_from(content).unwrap_or(0),
    );
    if let Some(page) = right_most {
        for (slot, byte) in out
            .iter_mut()
            .skip(start.saturating_add(8))
            .zip(page.to_be_bytes())
        {
            *slot = byte;
        }
    }
    for (at, offset) in pointers.iter().enumerate() {
        put16(
            &mut out,
            array.saturating_add(at.saturating_mul(2)),
            u16::try_from(*offset).unwrap_or(0),
        );
    }
    Some(out)
}

/// Two bytes at `at`, big-endian. A page shorter than that is one the
/// caller has already refused.
fn put16(out: &mut [u8], at: usize, value: u16) {
    for (slot, byte) in out.iter_mut().skip(at).zip(value.to_be_bytes()) {
        *slot = byte;
    }
}

/// A page changed where it lies.
///
/// This is the part of `MemPage` in `src/btree.c` that one page decides
/// for itself: `insertCell`, `dropCell`, `allocateSpace`, `freeSpace`,
/// `pageFindSlot` and `defragmentPage`. Which page a cell belongs on is
/// the balance of a tree and not of a page, and is not here.
///
/// The free space of a page is counted rather than carried, which is
/// `btreeComputeFreeSpace` on every call: O(f) for `f` freeblocks, where
/// SQLite keeps a number it has to keep right.
pub struct Writer<'a> {
    /// The page, reserved tail and all.
    bytes: &'a mut [u8],
    /// Where the b-tree header begins.
    start: usize,
    /// Bytes of the page the b-tree may use.
    usable: usize,
    /// What the page holds.
    kind: Kind,
}

impl<'a> Writer<'a> {
    /// Opens a page for changing.
    ///
    /// # Errors
    ///
    /// Whatever [`Page::parse`] refuses the page with.
    pub fn open(bytes: &'a mut [u8], number: u32, usable: u32) -> Result<Self, Error> {
        let kind = Page::parse(bytes, number, usable)?.kind();
        Ok(Writer {
            start: if number == 1 { HEADER_LEN } else { 0 },
            usable: size(u64::from(usable)),
            kind,
            bytes,
        })
    }

    /// The page as a reader sees it.
    #[must_use]
    pub const fn page(&self) -> Page<'_> {
        Page {
            bytes: self.bytes,
            start: self.start,
            usable: self.usable,
            kind: self.kind,
        }
    }

    /// Two bytes of the page, big-endian.
    fn get16(&self, at: usize) -> usize {
        usize::from(u16_at(self.bytes, at).unwrap_or(0))
    }

    /// Writes two bytes of the page, big-endian.
    fn put16(&mut self, at: usize, value: usize) {
        let bytes = u16::try_from(value).unwrap_or(0).to_be_bytes();
        for (slot, byte) in self.bytes.iter_mut().skip(at).zip(bytes) {
            *slot = byte;
        }
    }

    /// One byte of the page.
    fn get8(&self, at: usize) -> usize {
        usize::from(u8_at(self.bytes, at).unwrap_or(0))
    }

    /// Writes one byte of the page.
    fn put8(&mut self, at: usize, value: usize) {
        for slot in self.bytes.iter_mut().skip(at).take(1) {
            *slot = u8::try_from(value).unwrap_or(0);
        }
    }

    /// Copies `bytes` to `at`.
    fn put(&mut self, at: usize, bytes: &[u8]) {
        for (slot, byte) in self.bytes.iter_mut().skip(at).zip(bytes) {
            *slot = *byte;
        }
    }

    /// How many cells the page holds.
    fn cells(&self) -> usize {
        self.get16(self.start.saturating_add(3))
    }

    /// Where the cell pointer array begins.
    const fn array(&self) -> usize {
        self.start.saturating_add(self.kind.header_len())
    }

    /// Where the cell content area begins, with nought read as the whole
    /// of the largest page there is.
    fn content(&self) -> usize {
        match self.get16(self.start.saturating_add(5)) {
            0 => 65536,
            top => top,
        }
    }

    /// How many bytes of the page no cell holds, which is
    /// `btreeComputeFreeSpace`: the gap between the pointer array and the
    /// content, every freeblock, and the bytes too few to be one.
    ///
    /// # Errors
    ///
    /// [`Error::FreeBlock`] where the list leaves the page or does not
    /// count up.
    pub fn free(&self) -> Result<usize, Error> {
        let first = self.array().saturating_add(self.cells().saturating_mul(2));
        let top = self.content();
        if top < first || top > self.usable {
            return Err(Error::FreeBlock);
        }
        let mut free = top.saturating_sub(first);
        free = free.saturating_add(self.get8(self.start.saturating_add(7)));
        let mut at = self.get16(self.start.saturating_add(1));
        let mut last = 0;
        while at > 0 {
            if at <= last || at > self.usable.saturating_sub(4) {
                return Err(Error::FreeBlock);
            }
            let size = self.get16(at.saturating_add(2));
            if at.saturating_add(size) > self.usable {
                return Err(Error::FreeBlock);
            }
            free = free.saturating_add(size);
            last = at;
            at = self.get16(at);
        }
        if free > self.usable {
            return Err(Error::FreeBlock);
        }
        Ok(free)
    }

    /// Puts `size` bytes beginning at `at` back on the free list, which
    /// is `freeSpace`. Freeblocks that touch are made one, and a block at
    /// the content area moves the content area rather than joining the
    /// list.
    ///
    /// # Errors
    ///
    /// [`Error::FreeBlock`] where the list or the block leaves the page.
    pub(crate) fn release(&mut self, at: usize, size: usize) -> Result<(), Error> {
        let header = self.start;
        let mut start = at;
        let mut size = size;
        let mut end = start.saturating_add(size);
        let mut pointer = header.saturating_add(1);
        let mut block = 0;
        let mut fragments = 0;
        if self.get16(pointer) != 0 {
            loop {
                block = self.get16(pointer);
                if block >= start {
                    break;
                }
                if block <= pointer {
                    if block == 0 {
                        break;
                    }
                    return Err(Error::FreeBlock);
                }
                pointer = block;
            }
            if block > self.usable.saturating_sub(4) {
                return Err(Error::FreeBlock);
            }
            // A freeblock the released bytes reach is taken into them.
            if block != 0 && end.saturating_add(3) >= block {
                fragments = block.saturating_sub(end);
                if end > block {
                    return Err(Error::FreeBlock);
                }
                end = block.saturating_add(self.get16(block.saturating_add(2)));
                if end > self.usable {
                    return Err(Error::FreeBlock);
                }
                size = end.saturating_sub(start);
                block = self.get16(block);
            }
            // And the freeblock before them takes them, where it reaches.
            if pointer > header.saturating_add(1) {
                let above = pointer.saturating_add(self.get16(pointer.saturating_add(2)));
                if above.saturating_add(3) >= start {
                    if above > start {
                        return Err(Error::FreeBlock);
                    }
                    fragments = fragments.saturating_add(start.saturating_sub(above));
                    size = end.saturating_sub(pointer);
                    start = pointer;
                }
            }
            let held = self.get8(header.saturating_add(7));
            if fragments > held {
                return Err(Error::FreeBlock);
            }
            self.put8(header.saturating_add(7), held.saturating_sub(fragments));
        }
        let top = self.content();
        if start <= top {
            // The bytes are at the front of the content area, so the area
            // begins further down rather than the list growing.
            if start < top || pointer != header.saturating_add(1) {
                return Err(Error::FreeBlock);
            }
            self.put16(header.saturating_add(1), block);
            self.put16(header.saturating_add(5), end);
        } else {
            self.put16(pointer, start);
            self.put16(start, block);
            self.put16(start.saturating_add(2), size);
        }
        Ok(())
    }

    /// The freeblock `want` bytes fit in, taken off the list, which is
    /// `pageFindSlot`. A block between one and three bytes too large is
    /// taken whole and the extra counted as fragments, unless that would
    /// leave more fragments than a page may hold.
    ///
    /// # Errors
    ///
    /// [`Error::FreeBlock`] where the list leaves the page.
    pub(crate) fn slot(&mut self, want: usize) -> Result<Option<usize>, Error> {
        let header = self.start;
        let most = self.usable.saturating_sub(want);
        let mut pointer = header.saturating_add(1);
        let mut at = self.get16(pointer);
        // The caller asks only where the list holds something, so the
        // first block is past the header and every next one is past the
        // block that named it.
        while at <= most {
            let size = self.get16(at.saturating_add(2));
            if let Some(extra) = size.checked_sub(want) {
                if extra < 4 {
                    if self.get8(header.saturating_add(7)) > 57 {
                        return Ok(None);
                    }
                    let next = self.get16(at);
                    self.put16(pointer, next);
                    let held = self.get8(header.saturating_add(7));
                    self.put8(header.saturating_add(7), held.saturating_add(extra));
                    return Ok(Some(at));
                }
                if at.saturating_add(extra) > most {
                    return Err(Error::FreeBlock);
                }
                self.put16(at.saturating_add(2), extra);
                return Ok(Some(at.saturating_add(extra)));
            }
            pointer = at;
            at = self.get16(at);
            if at <= pointer {
                if at != 0 {
                    return Err(Error::FreeBlock);
                }
                return Ok(None);
            }
        }
        if at > most.saturating_add(want).saturating_sub(4) {
            return Err(Error::FreeBlock);
        }
        Ok(None)
    }

    /// Moves every cell to the end of the page, so that the free space is
    /// in one piece. This is `defragmentPage`.
    ///
    /// # Errors
    ///
    /// [`Error::FreeBlock`] where the page does not count up, and
    /// whatever reading a cell refuses.
    pub(crate) fn defragment(&mut self, most_fragments: usize) -> Result<(), Error> {
        let header = self.start;
        let cells = self.cells();
        let first = self.array().saturating_add(cells.saturating_mul(2));
        let free = self.free()?;
        let top = if self.get8(header.saturating_add(7)) <= most_fragments {
            self.closed()?
        } else {
            None
        };
        let top = match top {
            Some(top) => top,
            None => self.rewritten()?,
        };
        if self
            .get8(header.saturating_add(7))
            .saturating_add(top)
            .saturating_sub(first)
            != free
        {
            return Err(Error::FreeBlock);
        }
        self.put16(header.saturating_add(5), top);
        self.put16(header.saturating_add(1), 0);
        for at in first..top {
            self.put8(at, 0);
        }
        Ok(())
    }

    /// The one or two freeblocks of a page closed up by moving the
    /// content over them, which is the fast path of `defragmentPage`.
    /// Answers nothing where the page has more than two of them.
    fn closed(&mut self) -> Result<Option<usize>, Error> {
        let header = self.start;
        let free = self.get16(header.saturating_add(1));
        if free == 0 {
            return Ok(None);
        }
        // Every block of the list lies inside the page and ends inside
        // it, which `free` has just counted, so what is left to refuse is
        // a block the content area is above and a block another one
        // reaches into.
        let next = self.get16(free);
        if next != 0 && self.get16(next) != 0 {
            return Ok(None);
        }
        let mut size = self.get16(free.saturating_add(2));
        let top = self.content();
        if top >= free {
            return Err(Error::FreeBlock);
        }
        let mut second = 0;
        if next != 0 {
            if free.saturating_add(size) > next {
                return Err(Error::FreeBlock);
            }
            second = self.get16(next.saturating_add(2));
            self.slide(
                free.saturating_add(size),
                free.saturating_add(size).saturating_add(second),
                next.saturating_sub(free.saturating_add(size)),
            );
            size = size.saturating_add(second);
        }
        let broken = top.saturating_add(size);
        self.slide(top, broken, free.saturating_sub(top));
        let array = self.array();
        for at in 0..self.cells() {
            let pointer = array.saturating_add(at.saturating_mul(2));
            let offset = self.get16(pointer);
            if offset < free {
                self.put16(pointer, offset.saturating_add(size));
            } else if offset < next {
                self.put16(pointer, offset.saturating_add(second));
            }
        }
        Ok(Some(broken))
    }

    /// Every cell copied to the end of the page in the order the pointer
    /// array names them, which is the slow path of `defragmentPage`.
    fn rewritten(&mut self) -> Result<usize, Error> {
        let array = self.array();
        let cells = self.cells();
        let lowest = self.content();
        let mut top = self.usable;
        let mut held = Vec::with_capacity(cells);
        for at in 0..cells {
            let cell = self.page().cell(at)?;
            held.push(write_cell(&cell));
        }
        for (at, cell) in held.iter().enumerate() {
            top = top.checked_sub(cell.len()).ok_or(Error::FreeBlock)?;
            if top < lowest {
                return Err(Error::FreeBlock);
            }
            self.put(top, cell);
            self.put16(array.saturating_add(at.saturating_mul(2)), top);
        }
        self.put8(self.start.saturating_add(7), 0);
        Ok(top)
    }

    /// Moves `len` bytes from `from` to `to`, which may overlap.
    fn slide(&mut self, from: usize, to: usize, len: usize) {
        if to > from {
            for step in (0..len).rev() {
                let byte = self.get8(from.saturating_add(step));
                self.put8(to.saturating_add(step), byte);
            }
        } else {
            for step in 0..len {
                let byte = self.get8(from.saturating_add(step));
                self.put8(to.saturating_add(step), byte);
            }
        }
    }

    /// Where `want` bytes of cell content go, which is `allocateSpace`:
    /// off the free list where a block holds them, out of the gap
    /// otherwise, and after moving every cell to the end where the gap is
    /// too small.
    ///
    /// # Errors
    ///
    /// [`Error::FreeBlock`] where the page does not count up.
    pub(crate) fn allocate(&mut self, want: usize) -> Result<usize, Error> {
        let header = self.start;
        let gap = self.array().saturating_add(self.cells().saturating_mul(2));
        let mut top = self.content();
        if gap > top || top > self.usable {
            return Err(Error::FreeBlock);
        }
        if (self.get8(header.saturating_add(1)) != 0 || self.get8(header.saturating_add(2)) != 0)
            && gap.saturating_add(2) <= top
            && let Some(at) = self.slot(want)?
        {
            if at <= gap {
                return Err(Error::FreeBlock);
            }
            return Ok(at);
        }
        if gap.saturating_add(2).saturating_add(want) > top {
            let free = self.free()?;
            let most = free.saturating_sub(want.saturating_add(2)).min(4);
            self.defragment(most)?;
            top = self.content();
            if gap.saturating_add(2).saturating_add(want) > top {
                return Err(Error::FreeBlock);
            }
        }
        top = top.saturating_sub(want);
        self.put16(header.saturating_add(5), top);
        Ok(top)
    }

    /// Puts `cell` on the page as its cell number `at`.
    ///
    /// Answers `false` where the page has no room for it, which is the
    /// overflow cell of `insertCell` and is what a balance of the tree
    /// answers instead.
    ///
    /// # Errors
    ///
    /// [`Error::FreeBlock`] where the page does not count up.
    pub fn insert(&mut self, at: usize, cell: &[u8]) -> Result<bool, Error> {
        let cells = self.cells();
        if at > cells {
            return Err(Error::Overrun);
        }
        if cell.len().saturating_add(2) > self.free()? {
            return Ok(false);
        }
        let offset = self.allocate(cell.len())?;
        self.put(offset, cell);
        let array = self.array();
        let pointer = array.saturating_add(at.saturating_mul(2));
        self.slide(
            pointer,
            pointer.saturating_add(2),
            cells.saturating_sub(at).saturating_mul(2),
        );
        self.put16(pointer, offset);
        self.put16(self.start.saturating_add(3), cells.saturating_add(1));
        Ok(true)
    }

    /// Takes cell number `at` off the page, which is `dropCell`.
    ///
    /// # Errors
    ///
    /// [`Error::Overrun`] for a cell the page does not have, and
    /// [`Error::FreeBlock`] where the page does not count up.
    pub fn remove(&mut self, at: usize) -> Result<(), Error> {
        let cells = self.cells();
        if at >= cells {
            return Err(Error::Overrun);
        }
        let offset = self.page().cell_offset(at)?;
        // A cell the reader answers is one whose every byte it found
        // inside the page, and what is written back is never longer,
        // because a length the file wrote as a wide varint is written
        // back as the shortest one that holds it.
        let size = write_cell(&self.page().cell(at)?).len();
        self.release(offset, size)?;
        let header = self.start;
        if cells == 1 {
            self.put16(header.saturating_add(1), 0);
            self.put16(header.saturating_add(3), 0);
            self.put16(header.saturating_add(5), self.usable);
            self.put8(header.saturating_add(7), 0);
            return Ok(());
        }
        let array = self.array();
        let pointer = array.saturating_add(at.saturating_mul(2));
        self.slide(
            pointer.saturating_add(2),
            pointer,
            cells.saturating_sub(at).saturating_sub(1).saturating_mul(2),
        );
        self.put16(header.saturating_add(3), cells.saturating_sub(1));
        Ok(())
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
pub(crate) fn local_len(total: usize, usable: usize, kind: Kind) -> usize {
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
