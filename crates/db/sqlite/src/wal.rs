// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The write-ahead log a reader must follow.
//!
//! Section 4 of the format: a 32-byte header, then frames of a 24-byte
//! header and one page each. A reader takes the last valid frame of a
//! page that is a commit frame or is followed by one, and reads the page
//! from the database file only where the log holds no such frame.
//!
//! A frame is valid where its two salts are the header's and where its
//! two checksums are the running checksum over the header's first 24
//! bytes and, for every frame up to and including it, the frame header's
//! first 8 bytes and the whole page. The walk stops at the first frame
//! that fails either, because everything after it was written before the
//! last reset and is not this log.
//!
//! The `-shm` file is not read. Section 4.6 leaves its layout to the
//! host machine and says a reader rebuilds it from the log, which is
//! what [`Wal::open`] does: one pass of O(f) over `f` frames, and a
//! binary search of O(log p) over the `p` pages the log holds.

use alloc::vec::Vec;

use crate::bytes::{size, u32_at};
use crate::error::Error;

/// The length of the log's header.
const HEADER: usize = 32;

/// The length of a frame's header.
const FRAME: usize = 24;

/// What a log begins with when its checksum is computed little-endian.
const LITTLE: u32 = 0x377f_0682;

/// What a log begins with when its checksum is computed big-endian.
const BIG: u32 = 0x377f_0683;

/// A write-ahead log, with the newest committed frame of each page
/// already found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wal<'a> {
    /// The whole `-wal` file.
    bytes: &'a [u8],
    /// Each page the log holds, and which frame holds it newest, sorted
    /// by page number.
    newest: Vec<(u32, u32)>,
    /// The page size the log was written with, which the database
    /// header must agree with.
    page_size: u32,
    /// How many pages the database has after the last commit frame.
    pages: u32,
}

impl<'a> Wal<'a> {
    /// Reads a `-wal` file and finds the newest committed frame of every
    /// page it holds.
    ///
    /// A log of no valid frame answers a log that holds nothing, which
    /// is what a checkpointed or a reset log is.
    ///
    /// # Errors
    ///
    /// [`Error::Truncated`] for a file shorter than the header,
    /// [`Error::Magic`] for a magic number that is neither of the two,
    /// and [`Error::PageSize`] for a page size the format does not
    /// allow.
    pub fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        let head = bytes.get(..HEADER).ok_or(Error::Truncated)?;
        let word = |offset: usize| u32_at(head, offset).unwrap_or(0);
        let magic = word(0);
        if magic != LITTLE && magic != BIG {
            return Err(Error::Magic);
        }
        let page_size = word(8);
        if !page_size.is_power_of_two() || !(512..=65536).contains(&page_size) {
            return Err(Error::PageSize(page_size));
        }
        let salt = (word(16), word(20));
        let big = magic == BIG;
        // The running checksum begins with the header's own, which is
        // the checksum of the first 24 bytes of it.
        let mut running = (word(24), word(28));
        let mut newest: Vec<(u32, u32)> = Vec::new();
        let mut committed = 0usize;
        let mut pages = 0u32;
        let stride = size(u64::from(page_size)).saturating_add(FRAME);
        let mut at = HEADER;
        let mut index = 0u32;
        while let Some(frame) = bytes.get(at..at.saturating_add(stride)) {
            let field = |offset: usize| u32_at(frame, offset).unwrap_or(0);
            if (field(8), field(12)) != salt {
                break;
            }
            let carried = checksum(running, frame.get(..8).unwrap_or_default(), big);
            let carried = checksum(carried, frame.get(FRAME..).unwrap_or_default(), big);
            if carried != (field(16), field(20)) {
                break;
            }
            running = carried;
            place(&mut newest, field(0), index);
            if field(4) != 0 {
                // A commit frame ends a transaction, so everything up to
                // it is what a reader may see.
                committed = newest.len();
                pages = field(4);
            }
            index = index.saturating_add(1);
            at = at.saturating_add(stride);
        }
        newest.truncate(committed);
        Ok(Wal {
            bytes,
            newest,
            page_size,
            pages,
        })
    }

    /// The page size the log was written with.
    #[must_use]
    pub const fn page_size(&self) -> u32 {
        self.page_size
    }

    /// How many pages the database has after the last commit frame, or
    /// zero where the log holds no committed frame.
    #[must_use]
    pub const fn pages(&self) -> u32 {
        self.pages
    }

    /// The bytes of page `number` as the newest committed frame of it
    /// holds them, or nothing where the log holds no such frame.
    #[must_use]
    pub fn page_bytes(&self, number: u32) -> Option<&'a [u8]> {
        if number == 0 {
            // Section 1.2 numbers pages from one, so a frame that names
            // page zero names no page a reader can ask for.
            return None;
        }
        let at = self
            .newest
            .binary_search_by_key(&number, |(page, _)| *page)
            .ok()?;
        let (_, frame) = self.newest.get(at)?;
        let stride = size(u64::from(self.page_size)).saturating_add(FRAME);
        let start = HEADER
            .saturating_add(size(u64::from(*frame)).saturating_mul(stride))
            .saturating_add(FRAME);
        let end = start.saturating_add(size(u64::from(self.page_size)));
        self.bytes.get(start..end)
    }
}

/// Records that `frame` holds the newest copy of `page` so far.
///
/// The list is kept sorted by page number, so a page the log already
/// holds is found in O(log p) and a page it does not costs the shift of
/// the ones after it.
fn place(newest: &mut Vec<(u32, u32)>, page: u32, frame: u32) {
    match newest.binary_search_by_key(&page, |(held, _)| *held) {
        Ok(at) => {
            for slot in newest.iter_mut().skip(at).take(1) {
                *slot = (page, frame);
            }
        }
        Err(at) => newest.insert(at, (page, frame)),
    }
}

/// The checksum of section 4.2, carried on from `running`.
///
/// The input is read as 32-bit words in the byte order the magic names,
/// and a trailing run shorter than eight bytes takes no part, which no
/// caller has: a frame header's first 8 bytes and a page are both
/// multiples of eight.
fn checksum(running: (u32, u32), bytes: &[u8], big: bool) -> (u32, u32) {
    let (mut s0, mut s1) = running;
    let (pairs, _) = bytes.as_chunks::<8>();
    for pair in pairs {
        let (first, second) = pair.split_at(4);
        let word = |four: &[u8]| {
            let mut held = [0u8; 4];
            held.copy_from_slice(four);
            if big {
                u32::from_be_bytes(held)
            } else {
                u32::from_le_bytes(held)
            }
        };
        s0 = s0.wrapping_add(word(first)).wrapping_add(s1);
        s1 = s1.wrapping_add(word(second)).wrapping_add(s0);
    }
    (s0, s1)
}
