// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The pages of a database being written, and the trees on them.
//!
//! `src/btree.c` is the document. A row goes on the leaf its key belongs
//! to, which is the descent a reader makes; a payload longer than the
//! page holds runs onto overflow pages, which section 1.6 chains by
//! their first four bytes.
//!
//! What is not here is the balance: a leaf that will not hold a cell
//! answers that it will not, where SQLite splits it. Until that step a
//! table is as large as one page of it.

use alloc::vec::Vec;

use crate::bytes::size;
use crate::error::Error;
use crate::header::Header;
use crate::image::MAX_DEPTH;
use crate::page::{Cell, Kind, Page, Payload, Writer, build, local_len, write_cell};

/// The pages of a database being written, page one first.
pub struct Pages {
    /// Bytes per page.
    page_size: usize,
    /// Bytes of every page the b-tree may use.
    usable: usize,
    /// The pages themselves.
    held: Vec<Vec<u8>>,
}

impl Pages {
    /// A database of one page: page one, which is the leaf the schema
    /// table begins as.
    ///
    /// # Errors
    ///
    /// [`Error::PageSize`] for a page size the format does not allow and
    /// [`Error::Reserved`] for a tail that leaves too little of it.
    pub fn new(page_size: u32, reserved: u8) -> Result<Self, Error> {
        if !(512..=crate::header::MAX_PAGE_SIZE).contains(&page_size)
            || !page_size.is_power_of_two()
        {
            return Err(Error::PageSize(page_size));
        }
        let usable = page_size.saturating_sub(u32::from(reserved));
        if usable < crate::header::MIN_USABLE {
            return Err(Error::Reserved(reserved));
        }
        let page_size = size(u64::from(page_size));
        let usable = size(u64::from(usable));
        let first =
            build(Kind::LeafTable, 1, page_size, usable, &[], None).ok_or(Error::Overrun)?;
        Ok(Pages {
            page_size,
            usable,
            held: alloc::vec![first],
        })
    }

    /// How many pages the database holds.
    #[must_use]
    pub fn count(&self) -> u32 {
        u32::try_from(self.held.len()).unwrap_or(0)
    }

    /// Bytes of every page the b-tree may use.
    #[must_use]
    pub const fn usable(&self) -> usize {
        self.usable
    }

    /// A page of `kind` added at the end, and the number it has.
    ///
    /// # Errors
    ///
    /// [`Error::Overrun`] where the page is too small for the kind's own
    /// header.
    pub fn add(&mut self, kind: Kind) -> Result<u32, Error> {
        let number = self.count().saturating_add(1);
        let page =
            build(kind, number, self.page_size, self.usable, &[], None).ok_or(Error::Overrun)?;
        self.held.push(page);
        Ok(number)
    }

    /// A page of no kind at all, which is what an overflow page is, and
    /// the number it has.
    pub fn add_plain(&mut self) -> u32 {
        self.held.push(alloc::vec![0u8; self.page_size]);
        self.count()
    }

    /// One page, to read.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for a page the database does not hold, and
    /// whatever [`Page::parse`] refuses the page with.
    pub fn page(&self, number: u32) -> Result<Page<'_>, Error> {
        let bytes = self.bytes(number)?;
        Page::parse(bytes, number, u32::try_from(self.usable).unwrap_or(0))
    }

    /// One page, to change.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for a page the database does not hold, and
    /// whatever [`Writer::open`] refuses the page with.
    pub fn writer(&mut self, number: u32) -> Result<Writer<'_>, Error> {
        let usable = u32::try_from(self.usable).unwrap_or(0);
        let at = size(u64::from(number))
            .checked_sub(1)
            .ok_or(Error::Page(0))?;
        let bytes = self.held.get_mut(at).ok_or(Error::Page(number))?;
        Writer::open(bytes, number, usable)
    }

    /// The bytes of one page.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for a page the database does not hold.
    pub fn bytes(&self, number: u32) -> Result<&[u8], Error> {
        let at = size(u64::from(number))
            .checked_sub(1)
            .ok_or(Error::Page(0))?;
        self.held
            .get(at)
            .map(Vec::as_slice)
            .ok_or(Error::Page(number))
    }

    /// Writes a whole page, which is what a caller that built one out of
    /// its own bytes hands back.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for a page the database does not hold.
    pub fn put_page(&mut self, number: u32, bytes: &[u8]) -> Result<(), Error> {
        let at = size(u64::from(number))
            .checked_sub(1)
            .ok_or(Error::Page(0))?;
        let page = self.held.get_mut(at).ok_or(Error::Page(number))?;
        for (slot, byte) in page.iter_mut().zip(bytes) {
            *slot = *byte;
        }
        Ok(())
    }

    /// Writes `bytes` into page `number` at `at`. The callers write into
    /// a page they have just added, so the number is one the database
    /// holds.
    fn put(&mut self, number: u32, at: usize, bytes: &[u8]) {
        let index = size(u64::from(number)).saturating_sub(1);
        for page in self.held.iter_mut().skip(index).take(1) {
            for (slot, byte) in page.iter_mut().skip(at).zip(bytes) {
                *slot = *byte;
            }
        }
    }

    /// The file the pages make, with the header written into the first
    /// hundred bytes of page one.
    #[must_use]
    pub fn written(&self, header: &Header) -> Vec<u8> {
        crate::image::write(header, &self.held)
    }
}

/// What of a payload stays on the page, with the rest written onto
/// overflow pages.
fn spilled<'a>(pages: &mut Pages, payload: &'a [u8]) -> (&'a [u8], Option<u32>) {
    let local = local_len(payload.len(), pages.usable(), Kind::LeafTable);
    let held = payload.get(..local).unwrap_or_default();
    let mut rest = payload.get(local..).unwrap_or_default();
    if rest.is_empty() {
        return (held, None);
    }
    let span = pages.usable().saturating_sub(4);
    let first = pages.add_plain();
    let mut number = first;
    // The chain is walked once, and every page but the last is filled,
    // so the whole of it is O(n) in the bytes that run on.
    loop {
        let taken = rest.get(..span.min(rest.len())).unwrap_or_default();
        pages.put(number, 4, taken);
        rest = rest.get(taken.len()..).unwrap_or_default();
        if rest.is_empty() {
            break;
        }
        let next = pages.add_plain();
        pages.put(number, 0, &next.to_be_bytes());
        number = next;
    }
    (held, Some(first))
}

/// Puts one row in the table tree that begins at `root`.
///
/// A leaf that will not hold the cell is balanced: the root of a tree of
/// one page grows a child under it, and a right-most leaf that is full
/// grows a sibling beside it with a divider between them, which is
/// `balance_deeper` and `balance_quick`.
///
/// # Errors
///
/// [`Error::Balance`] for a cell that belongs anywhere but at the end of
/// the right-most page, which is the balance this crate does not write
/// yet; [`Error::Depth`] for a tree deeper than this crate walks; and
/// whatever reading a page of it refuses.
pub fn insert(pages: &mut Pages, root: u32, rowid: i64, record: &[u8]) -> Result<(), Error> {
    let (local, overflow) = spilled(pages, record);
    let cell = write_cell(&Cell::TableLeaf {
        rowid,
        payload: Payload {
            local,
            total: record.len(),
            overflow,
        },
    });
    let path = place(pages, root, rowid)?;
    let (leaf, at) = *path.last().ok_or(Error::Depth)?;
    if pages.writer(leaf)?.insert(at, &cell)? {
        return Ok(());
    }
    balance(pages, &path, &cell)
}

/// A leaf that will not hold a cell, balanced so that it does.
///
/// This is `balance` for the one shape this crate writes: a cell at the
/// end of the right-most page of its tree. The root of a tree of one
/// page grows a child under it, and the page that is full grows a
/// sibling beside it.
fn balance(pages: &mut Pages, path: &[(u32, usize)], cell: &[u8]) -> Result<(), Error> {
    let (leaf, at) = *path.last().ok_or(Error::Depth)?;
    if at != pages.page(leaf)?.cells() {
        return Err(Error::Balance);
    }
    // A tree of one page: the root keeps its place in the file, so its
    // cells move to a child and the root becomes the interior page above
    // it. The cell then belongs on the child.
    let (parent, above, page) = match path.split_last() {
        Some((_, [.., (parent, above)])) => (*parent, *above, leaf),
        _ => (leaf, 0, deepen(pages, leaf)?),
    };
    // The divider the sibling needs goes on the parent, so the parent has
    // to be an interior page with room for it, and the page has to be the
    // one every key above the last divider lives in.
    if parent == crate::image::SCHEMA_ROOT || above != pages.page(parent)?.cells() {
        return Err(Error::Balance);
    }
    quick(pages, parent, page, cell)
}

/// A root of one page grown into a root over one child, which is
/// `balance_deeper`: the child takes the root's content and the root
/// becomes the interior page above it.
pub(crate) fn deepen(pages: &mut Pages, root: u32) -> Result<u32, Error> {
    let kind = pages.page(root)?.kind();
    if kind != Kind::LeafTable || root == crate::image::SCHEMA_ROOT {
        return Err(Error::Balance);
    }
    let child = pages.add(kind)?;
    // `copyNodeContent`: the child takes the page the root was, header
    // and all, which it can because the two begin at the same byte of
    // their pages. A root on page one begins a hundred bytes in and is
    // the balance this crate does not write.
    let bytes = pages.bytes(root)?.to_vec();
    pages.put_page(child, &bytes)?;
    let mut above = pages.writer(root)?;
    above.zero(Kind::InteriorTable);
    above.point(child)?;
    Ok(child)
}

/// A full right-most leaf grown a sibling beside it, which is
/// `balance_quick`: the cell that did not fit is the whole of the new
/// page, and the parent gains a divider naming the page that was full
/// and the largest key on it.
pub(crate) fn quick(pages: &mut Pages, parent: u32, page: u32, cell: &[u8]) -> Result<(), Error> {
    let last = pages
        .page(page)?
        .cells()
        .checked_sub(1)
        .ok_or(Error::Balance)?;
    let Cell::TableLeaf { rowid, .. } = pages.page(page)?.cell(last)? else {
        return Err(Error::Balance);
    };
    let sibling = pages.add(Kind::LeafTable)?;
    if !pages.writer(sibling)?.insert(0, cell)? {
        return Err(Error::Balance);
    }
    let divider = write_cell(&Cell::TableInterior { child: page, rowid });
    let cells = pages.page(parent)?.cells();
    if !pages.writer(parent)?.insert(cells, &divider)? {
        return Err(Error::Balance);
    }
    pages.writer(parent)?.point(sibling)
}

/// The pages the descent to a key passes through, root first, each with
/// where the key belongs on it: the same descent a reader makes, and
/// then the first cell of the leaf whose key is above it.
fn place(pages: &Pages, root: u32, rowid: i64) -> Result<Vec<(u32, usize)>, Error> {
    let mut path = Vec::new();
    let mut number = root;
    for _ in 0..MAX_DEPTH {
        let page = pages.page(number)?;
        let cells = page.cells();
        let mut at = cells;
        for index in 0..cells {
            let above = match page.cell(index)? {
                Cell::TableInterior { rowid: key, .. } | Cell::TableLeaf { rowid: key, .. } => {
                    key >= rowid
                }
                _ => return Err(Error::PageKind(page.kind().byte())),
            };
            if above {
                at = index;
                break;
            }
        }
        path.push((number, at));
        if !page.kind().is_interior() {
            return Ok(path);
        }
        number = if at == cells {
            page.right_most().ok_or(Error::Overrun)?
        } else {
            page.child(at)?
        };
    }
    Err(Error::Depth)
}
