// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! OpenType `docs/microsoft/otff.html`, Table Directory and TTC Header.

use crate::{
    FontError,
    read::{self, Span, add, mul, offset},
};

/// Maximum faces in one collection.
pub const MAX_FACES: u32 = 64;
/// Maximum table records in one face.
pub const MAX_TABLES: u16 = 256;
/// Maximum total table records in one file, including shared records.
pub const MAX_TOTAL_TABLES: usize = 4096;

/// Outline format declared by the sfnt signature; payloads are not inspected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineKind {
    /// The sfnt signature is 0x00010000.
    TrueType,
    /// The sfnt signature is OTTO (CFF or CFF2).
    PostScript,
}

/// A bounded, uninterpreted table payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Table<'a> {
    /// Four-byte table identifier.
    pub tag: [u8; 4],
    /// Recorded checksum, not verified by envelope parsing.
    pub checksum: u32,
    /// Byte offset from the beginning of the complete font file.
    pub offset: u32,
    /// Actual table bytes, excluding alignment padding.
    pub data: &'a [u8],
}

impl<'a> Table<'a> {
    fn read(data: &'a [u8], record: &[u8]) -> Result<Self, FontError> {
        let start = read::u32(record, 8)?;
        let len = read::u32(record, 12)?;
        start.checked_add(len).ok_or(FontError::Overflow)?;
        Ok(Self {
            tag: read::array(record, 0)?,
            checksum: read::u32(record, 4)?,
            offset: start,
            data: read::bytes(data, offset(start)?, offset(len)?)?,
        })
    }

    fn span(self) -> Result<Span, FontError> {
        let start = offset(self.offset)?;
        Ok(Span {
            start,
            end: add(start, self.data.len())?,
        })
    }
}

/// One borrowed face in a validated sfnt or collection envelope.
#[derive(Clone, Copy, Debug)]
pub struct Font<'a> {
    data: &'a [u8],
    records: &'a [u8],
    directory: Span,
    count: u16,
    kind: OutlineKind,
}

impl<'a> Font<'a> {
    /// Validate the entire file and select face zero. Allocates nothing.
    ///
    /// # Errors
    /// Returns an envelope validation error; table contents remain uninterpreted.
    pub fn parse(data: &'a [u8]) -> Result<Self, FontError> {
        FontCollection::parse(data)?.font(0)
    }

    fn directory(data: &'a [u8], start: usize) -> Result<Self, FontError> {
        read::aligned(start)?;
        let header = read::bytes(data, start, 12)?;
        let kind = match read::u32(header, 0)? {
            0x0001_0000 => OutlineKind::TrueType,
            0x4f54_544f => OutlineKind::PostScript,
            _ => return Err(FontError::UnsupportedFormat),
        };
        let count = read::u16(header, 4)?;
        if count == 0 {
            return Err(FontError::EmptyDirectory);
        }
        if count > MAX_TABLES {
            return Err(FontError::LimitExceeded);
        }
        let len = mul(usize::from(count), 16)?;
        Ok(Self {
            data,
            records: read::bytes(data, add(start, 12)?, len)?,
            directory: Span::new(data, start, add(12, len)?)?,
            count,
            kind,
        })
    }

    /// The outline kind declared by the header.
    #[must_use]
    pub const fn outline_kind(self) -> OutlineKind {
        self.kind
    }

    /// Number of table records in this face.
    #[must_use]
    pub const fn table_count(self) -> u16 {
        self.count
    }

    /// Tables in ascending tag order, borrowing the original file.
    pub fn tables(self) -> impl Iterator<Item = Table<'a>> + Clone {
        // Every record and range was validated before this Font was exposed.
        self.records
            .as_chunks::<16>()
            .0
            .iter()
            .filter_map(move |r| Table::read(self.data, r).ok())
    }

    /// Find a tag in O(log T) time; absent tags return `None`.
    #[must_use]
    pub fn table(self, tag: [u8; 4]) -> Option<Table<'a>> {
        let mut low = 0usize;
        let mut high = usize::from(self.count);
        while low < high {
            let middle = low.checked_add(high.checked_sub(low)?.checked_div(2)?)?;
            let record = read::bytes(self.records, middle.checked_mul(16)?, 16).ok()?;
            let table = Table::read(self.data, record).ok()?;
            match table.tag.cmp(&tag) {
                core::cmp::Ordering::Less => low = middle.checked_add(1)?,
                core::cmp::Ordering::Equal => return Some(table),
                core::cmp::Ordering::Greater => high = middle,
            }
        }
        None
    }
}

/// A validated file containing one sfnt face or a TTC/OTC collection.
#[derive(Clone, Copy, Debug)]
pub struct FontCollection<'a> {
    data: &'a [u8],
    offsets: Option<&'a [u8]>,
    count: u32,
    header: Span,
    signature: Option<Span>,
}

impl<'a> FontCollection<'a> {
    /// Validate all directories and ranges without allocation or recursion.
    ///
    /// # Errors
    /// Rejects unsupported formats, invalid counts/tags/ranges, overlaps, and
    /// files exceeding the documented validation limits.
    pub fn parse(data: &'a [u8]) -> Result<Self, FontError> {
        let file = if read::array::<4>(data, 0)? == *b"ttcf" {
            Self::collection(data)?
        } else {
            Self {
                data,
                offsets: None,
                count: 1,
                header: Span { start: 0, end: 0 },
                signature: None,
            }
        };
        file.validate_directories()?;
        file.validate_tables()?;
        Ok(file)
    }

    fn collection(data: &'a [u8]) -> Result<Self, FontError> {
        let version = read::u32(data, 4)?;
        if !matches!(version, 0x0001_0000 | 0x0002_0000) {
            return Err(FontError::UnsupportedFormat);
        }
        let count = read::u32(data, 8)?;
        if count == 0 {
            return Err(FontError::EmptyDirectory);
        }
        if count > MAX_FACES {
            return Err(FontError::LimitExceeded);
        }
        let len = mul(offset(count)?, 4)?;
        let offsets = Some(read::bytes(data, 12, len)?);
        let end = add(12, len)?;
        let (header, signature) = if version == 0x0002_0000 {
            let fields = read::bytes(data, end, 12)?;
            let header = Span::new(data, 0, add(end, 12)?)?;
            let tag = read::u32(fields, 0)?;
            let len = read::u32(fields, 4)?;
            let start = read::u32(fields, 8)?;
            let signature = match (tag, len, start) {
                (0, 0, 0) => None,
                (0x4453_4947, 1.., _) => {
                    read::aligned(offset(start)?)?;
                    start.checked_add(len).ok_or(FontError::Overflow)?;
                    let span = Span::new(data, offset(start)?, offset(len)?)?;
                    if span.end != data.len() || span.intersects(header) {
                        return Err(FontError::InvalidSignature);
                    }
                    Some(span)
                }
                _ => return Err(FontError::InvalidSignature),
            };
            (header, signature)
        } else {
            (Span::new(data, 0, end)?, None)
        };
        Ok(Self {
            data,
            offsets,
            count,
            header,
            signature,
        })
    }

    /// Number of faces; always nonzero after successful parsing.
    #[must_use]
    pub const fn face_count(self) -> u32 {
        self.count
    }

    /// Select a face without repeating file validation.
    ///
    /// # Errors
    /// Returns `FontError::FaceIndex` if the index is out of range.
    pub fn font(self, index: u32) -> Result<Font<'a>, FontError> {
        if index >= self.count {
            return Err(FontError::FaceIndex);
        }
        let start = match self.offsets {
            Some(offsets) => offset(read::u32(offsets, mul(offset(index)?, 4)?)?)?,
            None => 0,
        };
        Font::directory(self.data, start)
    }

    fn validate_directories(self) -> Result<(), FontError> {
        let mut total = 0usize;
        for index in 0..self.count {
            let font = self.font(index)?;
            total = add(total, usize::from(font.count))?;
            if total > MAX_TOTAL_TABLES {
                return Err(FontError::LimitExceeded);
            }
            if font.directory.intersects(self.header)
                || self.signature.is_some_and(|s| s.intersects(font.directory))
            {
                return Err(FontError::Overlap);
            }
            for previous in 0..index {
                if font.directory.intersects(self.font(previous)?.directory) {
                    return Err(FontError::Overlap);
                }
            }
            let mut previous = None;
            for record in font.records.as_chunks::<16>().0 {
                let table = Table::read(self.data, record)?;
                validate_tag(table.tag)?;
                if previous.is_some_and(|tag| tag >= table.tag) {
                    return Err(FontError::TableOrder);
                }
                previous = Some(table.tag);
                read::aligned(offset(table.offset)?)?;
            }
        }
        Ok(())
    }

    fn validate_tables(self) -> Result<(), FontError> {
        for index in 0..self.count {
            let font = self.font(index)?;
            for (position, table) in font.tables().enumerate() {
                let span = table.span()?;
                if span.touches_metadata(self.header)
                    || self.signature.is_some_and(|s| span.touches_metadata(s))
                {
                    return Err(FontError::Overlap);
                }
                for other_index in 0..self.count {
                    let other = self.font(other_index)?;
                    if span.touches_metadata(other.directory) {
                        return Err(FontError::Overlap);
                    }
                    if other_index > index {
                        continue;
                    }
                    let count = if other_index == index {
                        position
                    } else {
                        usize::from(other.count)
                    };
                    for previous in other.tables().take(count) {
                        let previous_span = previous.span()?;
                        if span.intersects(previous_span)
                            && !(other_index != index
                                && span == previous_span
                                && table.tag == previous.tag
                                && table.checksum == previous.checksum)
                        {
                            return Err(FontError::Overlap);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn validate_tag(tag: [u8; 4]) -> Result<(), FontError> {
    let mut space = false;
    let mut nonspace = false;
    for byte in tag {
        if !(0x20..=0x7e).contains(&byte) || (space && byte != b' ') {
            return Err(FontError::InvalidTag);
        }
        if byte == b' ' {
            space = true;
        } else {
            nonspace = true;
        }
    }
    if nonspace {
        Ok(())
    } else {
        Err(FontError::InvalidTag)
    }
}
