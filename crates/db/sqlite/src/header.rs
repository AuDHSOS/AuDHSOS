// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The hundred bytes every database begins with.
//!
//! `docs/sqlite/fileformat2.html`, section 1.3, *The Database Header*, has
//! the table this reader follows, offset by offset.

use crate::bytes::{u8_at, u16_at, u32_at};
use crate::error::Error;

/// The string the file begins with, the nul included.
pub const MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Bytes of header, which are also the bytes page 1 carries before its
/// b-tree header.
pub const HEADER_LEN: usize = 100;

/// The smallest page the format allows.
pub const MIN_PAGE_SIZE: u32 = 512;

/// The largest page the format allows. In the header it is written as 1,
/// because 65536 does not fit the two bytes that hold it.
pub const MAX_PAGE_SIZE: u32 = 65536;

/// The fewest usable bytes a page may have after its reserved space.
pub const MIN_USABLE: u32 = 480;

/// How text is stored in this database.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// UTF-8.
    Utf8,
    /// UTF-16, little-endian.
    Utf16Le,
    /// UTF-16, big-endian.
    Utf16Be,
}

impl Encoding {
    /// The code the header holds for this encoding.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Encoding::Utf8 => 1,
            Encoding::Utf16Le => 2,
            Encoding::Utf16Be => 3,
        }
    }

    /// The encoding of a header code.
    ///
    /// # Errors
    ///
    /// [`Error::Encoding`] for anything but 1, 2 and 3.
    pub const fn from_code(code: u32) -> Result<Self, Error> {
        match code {
            1 => Ok(Encoding::Utf8),
            2 => Ok(Encoding::Utf16Le),
            3 => Ok(Encoding::Utf16Be),
            other => Err(Error::Encoding(other)),
        }
    }
}

/// The header of a database file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// Bytes per page, 512 to 65536 and a power of two.
    pub page_size: u32,
    /// The write version: 1 for a rollback journal, 2 for write-ahead
    /// logging.
    pub write_version: u8,
    /// The read version, read the same way.
    pub read_version: u8,
    /// Bytes at the end of every page that the b-tree layer does not use.
    pub reserved: u8,
    /// The counter the file bumps on every change.
    pub change_counter: u32,
    /// Pages in the file, as the header claims. Valid only when
    /// [`Self::version_valid_for`] equals the change counter.
    pub pages: u32,
    /// The first page of the free list, or zero.
    pub freelist: u32,
    /// How many pages the free list holds.
    pub freelist_pages: u32,
    /// The cookie that changes whenever the schema does.
    pub schema_cookie: u32,
    /// The schema format number, 1 to 4.
    pub schema_format: u32,
    /// The suggested page cache size.
    pub cache_size: u32,
    /// The largest root page when the file vacuums itself, else zero.
    pub largest_root: u32,
    /// How text is stored.
    pub encoding: Encoding,
    /// The number the application may set for itself.
    pub user_version: u32,
    /// Whether the file vacuums incrementally.
    pub incremental_vacuum: u32,
    /// The identifier an application may claim the file with.
    pub application_id: u32,
    /// The change counter the page count was written under.
    pub version_valid_for: u32,
    /// The library version that wrote the file last.
    pub library_version: u32,
}

impl Header {
    /// Bytes of a page the b-tree layer may use: the page without its
    /// reserved tail.
    #[must_use]
    pub fn usable(&self) -> u32 {
        self.page_size.saturating_sub(u32::from(self.reserved))
    }

    /// Reads the header out of the first hundred bytes of a file.
    ///
    /// # Errors
    ///
    /// [`Error::Truncated`] for fewer than a hundred bytes,
    /// [`Error::Magic`] for the wrong header string, and the errors of the
    /// three fields the format fixes: the page size, the reserved space
    /// and the payload fractions.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let head = bytes.get(..HEADER_LEN).ok_or(Error::Truncated)?;
        if head.get(..16) != Some(MAGIC.as_slice()) {
            return Err(Error::Magic);
        }
        let page_size = page_size(u16_at(head, 16).ok_or(Error::Truncated)?)?;
        let reserved = u8_at(head, 20).ok_or(Error::Truncated)?;
        if page_size.saturating_sub(u32::from(reserved)) < MIN_USABLE {
            return Err(Error::Reserved(reserved));
        }
        // Section 1.3 fixes these three at 64, 32 and 32; a file that says
        // anything else was written by something that is not SQLite.
        if (u8_at(head, 21), u8_at(head, 22), u8_at(head, 23)) != (Some(64), Some(32), Some(32)) {
            return Err(Error::Fractions);
        }
        let word = |offset: usize| u32_at(head, offset).ok_or(Error::Truncated);
        Ok(Header {
            page_size,
            write_version: u8_at(head, 18).ok_or(Error::Truncated)?,
            read_version: u8_at(head, 19).ok_or(Error::Truncated)?,
            reserved,
            change_counter: word(24)?,
            pages: word(28)?,
            freelist: word(32)?,
            freelist_pages: word(36)?,
            schema_cookie: word(40)?,
            schema_format: word(44)?,
            cache_size: word(48)?,
            largest_root: word(52)?,
            encoding: Encoding::from_code(word(56)?)?,
            user_version: word(60)?,
            incremental_vacuum: word(64)?,
            application_id: word(68)?,
            version_valid_for: word(92)?,
            library_version: word(96)?,
        })
    }
}

/// The page size a two-byte header field stands for. The value 1 means
/// 65536, which section 1.3 writes that way because the field is two bytes
/// wide.
fn page_size(field: u16) -> Result<u32, Error> {
    let size = if field == 1 {
        MAX_PAGE_SIZE
    } else {
        u32::from(field)
    };
    if !(MIN_PAGE_SIZE..=MAX_PAGE_SIZE).contains(&size) || !size.is_power_of_two() {
        return Err(Error::PageSize(size));
    }
    Ok(size)
}
