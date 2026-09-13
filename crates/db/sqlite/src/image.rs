// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A whole database, read where it lies: the pages of a byte slice, the
//! b-trees over them, and the schema that names the rest.

use crate::bytes::{size, u32_at};
use crate::error::Error;
use crate::header::Header;
use crate::page::{Kind, Page, Payload};
use crate::record::Record;

/// How deep a tree this crate walks. SQLite's own cursor stops at twenty;
/// a tree deeper than this is a file that points into itself.
pub const MAX_DEPTH: usize = 32;

/// The page the schema table begins at.
pub const SCHEMA_ROOT: u32 = 1;

/// A database file, read without being copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Image<'a> {
    /// The whole file.
    bytes: &'a [u8],
    /// Its header.
    header: Header,
}

impl<'a> Image<'a> {
    /// Reads the header of a file and leaves the rest where it is.
    ///
    /// # Errors
    ///
    /// The errors of [`Header::parse`].
    pub fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        let header = Header::parse(bytes)?;
        Ok(Image { bytes, header })
    }

    /// The header.
    #[must_use]
    pub const fn header(&self) -> &Header {
        &self.header
    }

    /// How many whole pages the file holds. This is the file's length and
    /// not the header's claim: the two differ while a transaction is being
    /// written, and what can be read is what is there.
    #[must_use]
    pub fn pages(&self) -> u32 {
        let size = u64::try_from(self.bytes.len()).unwrap_or(0);
        size.checked_div(u64::from(self.header.page_size))
            .and_then(|pages| u32::try_from(pages).ok())
            .unwrap_or(0)
    }

    /// The bytes of one page, numbered from one.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for page zero or a page past the end of the file.
    pub fn page_bytes(&self, number: u32) -> Result<&'a [u8], Error> {
        if number == 0 {
            return Err(Error::Page(0));
        }
        let page_size = size(u64::from(self.header.page_size));
        let start = size(u64::from(number.saturating_sub(1))).saturating_mul(page_size);
        let end = start.saturating_add(page_size);
        self.bytes.get(start..end).ok_or(Error::Page(number))
    }

    /// One b-tree page.
    ///
    /// # Errors
    ///
    /// The errors of [`Image::page_bytes`] and of [`Page::parse`].
    pub fn page(&self, number: u32) -> Result<Page<'a>, Error> {
        Page::parse(self.page_bytes(number)?, number, self.header.usable())
    }

    /// The rows of the table whose tree begins at `root`, in rowid order.
    #[must_use]
    pub const fn rows(&self, root: u32) -> Rows<'a> {
        Rows::new(*self, root)
    }

    /// The rows of the schema table, which is the tree at page 1: one row
    /// per table, index, view and trigger, as section 2.6 describes.
    #[must_use]
    pub const fn schema(&self) -> Rows<'a> {
        self.rows(SCHEMA_ROOT)
    }

    /// Copies a payload into `into`, following its overflow chain, and
    /// answers how many bytes it wrote.
    ///
    /// # Errors
    ///
    /// [`Error::Overrun`] where `into` is shorter than the payload, and
    /// [`Error::Overflow`] for a chain that ends early or turns back on
    /// itself.
    pub fn read_payload(&self, payload: &Payload<'a>, into: &mut [u8]) -> Result<usize, Error> {
        let room: &mut [u8] = into.get_mut(..payload.total).ok_or(Error::Overrun)?;
        let (mut room, mut written) = copy(room, payload.local);
        let mut next = payload.overflow;
        let mut steps = 0u32;
        while let Some(number) = next {
            // The chain can be no longer than the file has pages; a longer
            // walk is a chain that turned back on itself.
            steps = steps.saturating_add(1);
            if steps > self.pages() {
                return Err(Error::Overflow(number));
            }
            let page = self.page_bytes(number)?;
            let usable = size(u64::from(self.header.usable()));
            // Every page is at least as long as the usable part of one, so
            // what is left after the link is what the chain carries.
            let content = page.get(4..usable).unwrap_or_default();
            let (rest, added) = copy(room, content);
            room = rest;
            written = written.saturating_add(added);
            next = match u32_at(page, 0).unwrap_or(0) {
                0 => None,
                further => Some(further),
            };
            if written >= payload.total {
                break;
            }
        }
        if written < payload.total {
            return Err(Error::Overflow(payload.overflow.unwrap_or(0)));
        }
        Ok(payload.total)
    }
}

/// Copies as much of `from` into the front of `room` as both allow, and
/// answers what is left of `room` together with how much was written.
fn copy<'b>(room: &'b mut [u8], from: &[u8]) -> (&'b mut [u8], usize) {
    let len = room.len().min(from.len());
    let (target, rest) = room.split_at_mut(len);
    for (slot, byte) in target.iter_mut().zip(from) {
        *slot = *byte;
    }
    (rest, len)
}

/// Where a walk stands on one page.
#[derive(Clone, Copy, Debug)]
struct Frame {
    /// The page.
    number: u32,
    /// The next cell to read, which for an interior page counts the
    /// right-most pointer as one past the last cell.
    next: usize,
}

/// The rows of one table tree, in rowid order.
///
/// The walk keeps one frame per level and no allocation: a tree of `n`
/// rows costs O(n) steps and O(depth) memory, and depth is bounded by
/// [`MAX_DEPTH`].
#[derive(Clone, Debug)]
pub struct Rows<'a> {
    /// The file being read.
    image: Image<'a>,
    /// The path from the root to where the walk stands.
    stack: [Frame; MAX_DEPTH],
    /// How much of the path is in use.
    depth: usize,
    /// Whether a refusal has ended the walk.
    done: bool,
}

impl<'a> Rows<'a> {
    /// A walk that has not begun, over the tree at `root`.
    const fn new(image: Image<'a>, root: u32) -> Self {
        Rows {
            image,
            stack: [Frame {
                number: root,
                next: 0,
            }; MAX_DEPTH],
            depth: 1,
            done: false,
        }
    }

    /// Ends the walk and answers the refusal that ended it.
    const fn stop(&mut self, error: Error) -> Error {
        self.done = true;
        error
    }

    /// The frame the walk stands on, or nothing once it has climbed out.
    fn top(&mut self) -> Option<&mut Frame> {
        self.stack.iter_mut().take(self.depth).last()
    }

    /// Descends into `child`.
    fn push(&mut self, child: u32) -> Result<(), Error> {
        let slot = self.stack.get_mut(self.depth).ok_or(Error::Depth)?;
        *slot = Frame {
            number: child,
            next: 0,
        };
        self.depth = self.depth.saturating_add(1);
        Ok(())
    }
}

/// One row of a table: its key and its record, whole or with the rest on
/// overflow pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row<'a> {
    /// The rowid.
    pub rowid: i64,
    /// The record, as it lies.
    pub payload: Payload<'a>,
}

impl<'a> Row<'a> {
    /// The record, for a row that is whole.
    ///
    /// # Errors
    ///
    /// [`Error::Overrun`] where the row continues on an overflow page, in
    /// which case [`Image::read_payload`] is what reads it, and the errors
    /// of [`Record::parse`].
    pub fn record(&self) -> Result<Record<'a>, Error> {
        if !self.payload.is_whole() {
            return Err(Error::Overrun);
        }
        Record::parse(self.payload.local)
    }
}

impl<'a> Iterator for Rows<'a> {
    type Item = Result<Row<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            let frame = *self.top()?;
            let page = match self.image.page(frame.number) {
                Ok(page) => page,
                Err(error) => return Some(Err(self.stop(error))),
            };
            let cells = page.cells();
            match page.kind() {
                Kind::LeafTable if frame.next < cells => {
                    self.bump();
                    return Some(match page.row(frame.next) {
                        Ok((rowid, payload)) => Ok(Row { rowid, payload }),
                        Err(error) => Err(self.stop(error)),
                    });
                }
                Kind::InteriorTable if frame.next <= cells => {
                    self.bump();
                    let child = if frame.next == cells {
                        page.right_most().ok_or(Error::Overrun)
                    } else {
                        page.child(frame.next)
                    };
                    match child.and_then(|child| self.push(child)) {
                        Ok(()) => {}
                        Err(error) => return Some(Err(self.stop(error))),
                    }
                }
                Kind::InteriorIndex | Kind::LeafIndex => {
                    // A table tree holds no index page; a root that leads
                    // to one is a root of the wrong tree.
                    return Some(Err(self.stop(Error::PageKind(page.kind().byte()))));
                }
                Kind::InteriorTable | Kind::LeafTable => {
                    self.depth = self.depth.saturating_sub(1);
                }
            }
        }
        None
    }
}

impl Rows<'_> {
    /// Moves the top frame on to its next cell.
    ///
    /// The frame is reached as a walk of at most one rather than through
    /// an `if let`: every caller stands on a frame, so the `else` of an
    /// `if let` here could never run, and a branch no input reaches is one
    /// no test can hold to anything.
    fn bump(&mut self) {
        let index = self.depth.saturating_sub(1);
        for frame in self.stack.iter_mut().skip(index).take(1) {
            frame.next = frame.next.saturating_add(1);
        }
    }
}
