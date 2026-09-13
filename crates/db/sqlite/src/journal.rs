// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The rollback journal a reader must play back.
//!
//! A `-journal` file beside a database holds the content each page had
//! before a transaction. Section 1.1 of the format calls such a file hot
//! where it holds what recovering the database needs, and says a reader
//! cannot ignore one: the database file is then a transaction half
//! written, and reading it answers rows no commit ever made.
//!
//! The format is not in `docs/sqlite/fileformat2.html`, which describes
//! the main file and the write-ahead log; it is in `src/pager.c`, at
//! `writeJournalHdr`, `readJournalHdr`, `pager_cksum` and
//! `pager_playback`. A header of one sector holds the magic, how many
//! records follow, the nonce each record's checksum starts from, how
//! many pages the database had, the sector size and the page size.
//! Records of a page number, a page and a checksum follow it, and
//! another header may follow those.
//!
//! [`Journal::open`] plays the records back in memory rather than over
//! the file: one pass of O(r) over `r` records, and a lookup of
//! O(log p) over the `p` pages it restores. A page written twice keeps
//! the first record, which is the content the transaction started from,
//! and is the `pDone` bitvec of `pager_playback_one_page`.

use alloc::vec::Vec;

use crate::bytes::{size, u32_at};
use crate::error::Error;

/// What a journal begins with once its records are on disk. A journal
/// whose first eight bytes are anything else was written by a process
/// that had not yet synced them, and the database it sits beside was
/// never changed.
const MAGIC: [u8; 8] = [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7];

/// The record count a process working without sync writes, which says
/// the rest of the file is records and no header follows them.
const TO_THE_END: u32 = 0xffff_ffff;

/// The smallest sector a header is padded to, which `readJournalHdr`
/// refuses below.
const MIN_SECTOR: u32 = 32;

/// The largest, which is `MAX_SECTOR_SIZE` of `src/pager.c`.
const MAX_SECTOR: u32 = 0x0100_0000;

/// A rollback journal, with the content it restores already found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Journal<'a> {
    /// The whole `-journal` file.
    bytes: &'a [u8],
    /// Each page the journal restores, and where its record's page
    /// begins, sorted by page number.
    restored: Vec<(u32, usize)>,
    /// The page size the journal was written with.
    page_size: u32,
    /// How many pages the database had when the transaction began.
    pages: u32,
}

impl<'a> Journal<'a> {
    /// Reads a `-journal` file and plays it back in memory.
    ///
    /// A file that does not begin with the magic answers a journal that
    /// restores nothing, because such a file is not hot: `readJournalHdr`
    /// stops there and `pager_playback` writes no page.
    ///
    /// # Errors
    ///
    /// [`Error::Truncated`] for a file shorter than the eight bytes the
    /// magic takes.
    pub fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        let head = bytes.get(..8).ok_or(Error::Truncated)?;
        let mut journal = Journal {
            bytes,
            restored: Vec::new(),
            page_size: 0,
            pages: 0,
        };
        if head != MAGIC {
            return Ok(journal);
        }
        journal.play();
        Ok(journal)
    }

    /// The page size the journal was written with, or zero where it
    /// restores nothing.
    #[must_use]
    pub const fn page_size(&self) -> u32 {
        self.page_size
    }

    /// How many pages the database had when the transaction began, or
    /// zero where the journal restores nothing.
    #[must_use]
    pub const fn pages(&self) -> u32 {
        self.pages
    }

    /// Whether the journal holds what recovering the database needs,
    /// which section 1.1 calls hot.
    #[must_use]
    pub const fn hot(&self) -> bool {
        self.pages != 0
    }

    /// The bytes page `number` had before the transaction, or nothing
    /// where the journal restores no such page.
    #[must_use]
    pub fn page_bytes(&self, number: u32) -> Option<&'a [u8]> {
        if number == 0 {
            // Section 1.2 numbers pages from one.
            return None;
        }
        let at = self
            .restored
            .binary_search_by_key(&number, |(page, _)| *page)
            .ok()?;
        let (_, start) = self.restored.get(at)?;
        let end = start.saturating_add(size(u64::from(self.page_size)));
        self.bytes.get(*start..end)
    }

    /// Reads every header of the journal and the records under it.
    fn play(&mut self) {
        let mut at = 0usize;
        while at < self.bytes.len() {
            let Some(header) = self.header(at) else {
                return;
            };
            let stride = size(u64::from(header.page_size)).saturating_add(8);
            let mut record = at.saturating_add(size(u64::from(header.sector)));
            for _ in 0..header.records(self.bytes.len(), record, stride) {
                if !self.record(record, &header) {
                    return;
                }
                record = record.saturating_add(stride);
            }
            // Another header may follow, at the next sector boundary.
            let sector = size(u64::from(header.sector));
            let over = record.checked_rem(sector).unwrap_or(0);
            at = match over {
                0 => record,
                over => record.saturating_add(sector.saturating_sub(over)),
            };
        }
    }

    /// Reads one header, or nothing where the bytes at `at` are not one.
    fn header(&mut self, at: usize) -> Option<Header> {
        let head = self.bytes.get(at..at.saturating_add(28))?;
        if head.get(..8)? != MAGIC {
            return None;
        }
        let word = |offset: usize| u32_at(head, offset).unwrap_or(0);
        let (sector, page_size) = (word(20), word(24));
        // `readJournalHdr` refuses either outside its range or not a
        // power of two, because a process that wrote one had crashed
        // before the header was synced.
        if !(MIN_SECTOR..=MAX_SECTOR).contains(&sector)
            || !sector.is_power_of_two()
            || !(512..=65536).contains(&page_size)
            || !page_size.is_power_of_two()
        {
            return None;
        }
        if self.page_size == 0 {
            // The first header is the one that says how large the
            // database was and how large a page is; the rest repeat it.
            self.page_size = page_size;
            self.pages = word(16);
        }
        Some(Header {
            count: word(8),
            nonce: word(12),
            sector,
            page_size,
        })
    }

    /// Reads one record, and answers whether the walk goes on.
    fn record(&mut self, at: usize, header: &Header) -> bool {
        let page_size = size(u64::from(header.page_size));
        let Some(field) = self.bytes.get(at..at.saturating_add(4)) else {
            return false;
        };
        let number = u32_at(field, 0).unwrap_or(0);
        let start = at.saturating_add(4);
        let end = start.saturating_add(page_size);
        let Some(page) = self.bytes.get(start..end) else {
            return false;
        };
        let Some(field) = self.bytes.get(end..end.saturating_add(4)) else {
            return false;
        };
        if number == 0 {
            // `pager_playback_one_page` stops on a record naming page
            // zero, which is garbage left where a journal was cut short.
            return false;
        }
        if checksum(header.nonce, page) != u32_at(field, 0).unwrap_or(0) {
            return false;
        }
        if number <= self.pages {
            // A page past the database's own size is a record the
            // playback passes over; the truncation restores it.
            place(&mut self.restored, number, start);
        }
        true
    }
}

/// One header of a journal.
struct Header {
    /// How many records follow, or [`TO_THE_END`].
    count: u32,
    /// What each record's checksum starts from.
    nonce: u32,
    /// What the header is padded to, and what the next one begins at.
    sector: u32,
    /// The page size the records hold.
    page_size: u32,
}

impl Header {
    /// How many records follow this header.
    ///
    /// A count of [`TO_THE_END`] means the rest of the file is records,
    /// which is what `pager_playback` computes from the file's length.
    fn records(&self, len: usize, from: usize, stride: usize) -> u32 {
        if self.count != TO_THE_END {
            return self.count;
        }
        let room = len.saturating_sub(from);
        let count = room.checked_div(stride).unwrap_or(0);
        u32::try_from(count).unwrap_or(u32::MAX)
    }
}

/// Records that the record at `start` holds what `page` began with.
///
/// The first record of a page is the one that is kept, because a page
/// journaled twice was journaled first with the content the transaction
/// started from. This is the `pDone` bitvec of
/// `pager_playback_one_page`.
fn place(restored: &mut Vec<(u32, usize)>, page: u32, start: usize) {
    if let Err(at) = restored.binary_search_by_key(&page, |(held, _)| *held) {
        restored.insert(at, (page, start));
    }
}

/// The checksum of one record, which is `pager_cksum` of `src/pager.c`:
/// the nonce plus every two-hundredth byte, counting back from two
/// hundred short of the page's end.
fn checksum(nonce: u32, page: &[u8]) -> u32 {
    let mut sum = nonce;
    let mut at = page.len().checked_sub(200);
    while let Some(index) = at.filter(|index| *index > 0) {
        sum = sum.wrapping_add(u32::from(page.get(index).copied().unwrap_or(0)));
        at = index.checked_sub(200);
    }
    sum
}
