// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Validated Unicode maps. Source: `docs/microsoft/cmap.html`, formats 0/4/6/12/13/14.

use crate::{
    FontError,
    read::{self, add, mul, offset, sub},
};

/// A selected Unicode mapping and optional variation-sequence supplement.
#[derive(Clone, Copy, Debug)]
pub struct Cmap<'a> {
    primary: Subtable<'a>,
    variations: Option<Variations<'a>>,
}

impl<'a> Cmap<'a> {
    /// Select and validate a Unicode mapping against `num_glyphs`.
    ///
    /// Preference: formats 12, 13, 4, 6, 0; Unicode platform before Windows
    /// for equal formats, then higher encoding ID. Only one primary is used.
    ///
    /// # Errors
    /// Rejects malformed selected tables, invalid glyph IDs, more than 64
    /// encoding records, and the absence of a supported Unicode mapping.
    pub fn parse(data: &'a [u8], num_glyphs: u16) -> Result<Self, FontError> {
        if read::u16(data, 0)? != 0 || num_glyphs == 0 {
            return Err(FontError::InvalidTable);
        }
        let count = usize::from(read::u16(data, 2)?);
        if count > 64 {
            return Err(FontError::LimitExceeded);
        }
        let end = add(4, mul(count, 8)?)?;
        let records = read::bytes(data, 4, mul(count, 8)?)?;
        let mut selected = None;
        let mut selected_rank = (0, 0, 0);
        let mut variation_data = None;
        let mut previous = None;
        for record in records.as_chunks::<8>().0 {
            let platform = read::u16(record, 0)?;
            let encoding = read::u16(record, 2)?;
            let pair = (platform, encoding);
            if previous.is_some_and(|p| p >= pair) {
                return Err(FontError::TableOrder);
            }
            previous = Some(pair);
            let start = offset(read::u32(record, 4)?)?;
            if start < end {
                return Err(FontError::Overlap);
            }
            let table = read::tail(data, start)?;
            let format = read::u16(table, 0)?;
            if platform == 0 && encoding == 5 {
                if format != 14 {
                    return Err(FontError::InvalidTable);
                }
                variation_data = Some(table);
                continue;
            }
            if format == 14 {
                return Err(FontError::InvalidTable);
            }
            if !((platform == 0 && matches!(encoding, 0..=4 | 6))
                || (platform == 3 && matches!(encoding, 1 | 10)))
            {
                continue;
            }
            let priority = match format {
                12 => 5,
                13 => 4,
                4 => 3,
                6 => 2,
                0 => 1,
                _ => continue,
            };
            let rank = (priority, u8::from(platform == 0), encoding);
            if rank > selected_rank {
                selected = Some(table);
                selected_rank = rank;
            }
        }
        Ok(Self {
            primary: Subtable::parse(selected.ok_or(FontError::UnsupportedFormat)?, num_glyphs)?,
            variations: variation_data
                .map(|data| Variations::parse(data, num_glyphs))
                .transpose()?,
        })
    }

    /// The selected primary format.
    #[must_use]
    pub const fn format(self) -> u16 {
        self.primary.format
    }

    /// Map a scalar value; zero denotes the missing glyph.
    #[must_use]
    pub fn glyph_index(self, character: char) -> u16 {
        self.primary.lookup(u32::from(character)).unwrap_or(0)
    }

    /// Map a supported variation sequence; `None` means unsupported.
    /// A supported sequence may explicitly map to glyph zero.
    #[must_use]
    pub fn variation_glyph(self, character: char, selector: char) -> Option<u16> {
        self.variations?
            .lookup(u32::from(character), u32::from(selector), self.primary)
            .ok()
            .flatten()
    }
}

#[derive(Clone, Copy, Debug)]
struct Subtable<'a> {
    data: &'a [u8],
    format: u16,
    count: usize,
}

fn glyph(value: u32, count: u16) -> Result<u16, FontError> {
    if value >= u32::from(count) {
        return Err(FontError::GlyphIndex);
    }
    u16::try_from(value).map_err(|_| FontError::GlyphIndex)
}

impl<'a> Subtable<'a> {
    fn parse(data: &'a [u8], glyphs: u16) -> Result<Self, FontError> {
        let format = read::u16(data, 0)?;
        let length = match format {
            0 | 4 | 6 => usize::from(read::u16(data, 2)?),
            12 | 13 => offset(read::u32(data, 4)?)?,
            _ => return Err(FontError::UnsupportedFormat),
        };
        let data = read::bytes(data, 0, length)?;
        let count = match format {
            0 => {
                if length != 262 || read::u16(data, 4)? != 0 {
                    return Err(FontError::InvalidTable);
                }
                for &id in read::bytes(data, 6, 256)? {
                    glyph(u32::from(id), glyphs)?;
                }
                256
            }
            4 => Self::validate_four(data, glyphs)?,
            6 => {
                let first = usize::from(read::u16(data, 6)?);
                let count = usize::from(read::u16(data, 8)?);
                if add(first, count)? > 65536
                    || read::u16(data, 4)? != 0
                    || length != add(10, mul(count, 2)?)?
                {
                    return Err(FontError::InvalidTable);
                }
                for entry in read::bytes(data, 10, mul(count, 2)?)?.as_chunks::<2>().0 {
                    glyph(u32::from(u16::from_be_bytes(*entry)), glyphs)?;
                }
                count
            }
            12 | 13 => {
                if read::u16(data, 2)? != 0 || read::u32(data, 8)? != 0 {
                    return Err(FontError::InvalidTable);
                }
                let count = offset(read::u32(data, 12)?)?;
                if length != add(16, mul(count, 12)?)? {
                    return Err(FontError::InvalidTable);
                }
                let mut previous = None;
                for group in read::bytes(data, 16, mul(count, 12)?)?.as_chunks::<12>().0 {
                    let start = read::u32(group, 0)?;
                    let end = read::u32(group, 4)?;
                    let id = read::u32(group, 8)?;
                    if start > end || end > 0x0010_ffff || previous.is_some_and(|p| p >= start) {
                        return Err(FontError::InvalidTable);
                    }
                    let last = if format == 12 {
                        id.checked_add(end.checked_sub(start).ok_or(FontError::InvalidTable)?)
                            .ok_or(FontError::Overflow)?
                    } else {
                        id
                    };
                    glyph(last, glyphs)?;
                    previous = Some(end);
                }
                count
            }
            _ => return Err(FontError::UnsupportedFormat),
        };
        Ok(Self {
            data,
            format,
            count,
        })
    }

    fn validate_four(data: &'a [u8], glyphs: u16) -> Result<usize, FontError> {
        let length = data.len();
        let format = 4;
        let doubled = usize::from(read::u16(data, 6)?);
        if doubled == 0
            || !doubled.is_multiple_of(2)
            || !length.is_multiple_of(2)
            || read::u16(data, 4)? != 0
        {
            return Err(FontError::InvalidTable);
        }
        let count = doubled / 2;
        read::bytes(data, 0, add(16, mul(count, 8)?)?)?;
        if read::u16(data, add(14, doubled)?)? != 0 {
            return Err(FontError::InvalidTable);
        }
        let table = Self {
            data,
            format,
            count,
        };
        let mut previous = None;
        for i in 0..count {
            let (start, end, _, range) = table.segment(i)?;
            if start > end || previous.is_some_and(|p| p >= start) {
                return Err(FontError::InvalidTable);
            }
            if range != 0 {
                let at = table.glyph_array_offset(i, start)?;
                if at < add(16, mul(count, 8)?)? || !range.is_multiple_of(2) {
                    return Err(FontError::InvalidTable);
                }
                read::u16(data, table.glyph_array_offset(i, end)?)?;
            }
            for value in start..=end {
                glyph(u32::from(table.segment_glyph(i, value)?), glyphs)?;
            }
            previous = Some(end);
        }
        let (start, end, _, _) = table.segment(sub(count, 1)?)?;
        if start != 0xffff || end != 0xffff {
            return Err(FontError::InvalidTable);
        }
        Ok(count)
    }

    fn segment(self, index: usize) -> Result<(u16, u16, u16, u16), FontError> {
        let i = mul(index, 2)?;
        let end = read::u16(self.data, add(14, i)?)?;
        let start = read::u16(self.data, add(add(16, mul(self.count, 2)?)?, i)?)?;
        let delta = read::u16(self.data, add(add(16, mul(self.count, 4)?)?, i)?)?;
        let range = read::u16(self.data, add(add(16, mul(self.count, 6)?)?, i)?)?;
        Ok((start, end, delta, range))
    }

    fn glyph_array_offset(self, index: usize, value: u16) -> Result<usize, FontError> {
        let (start, _, _, range) = self.segment(index)?;
        add(
            add(add(16, mul(self.count, 6)?)?, mul(index, 2)?)?,
            add(
                usize::from(range),
                mul(sub(usize::from(value), usize::from(start))?, 2)?,
            )?,
        )
    }

    fn segment_glyph(self, index: usize, value: u16) -> Result<u16, FontError> {
        let (_, _, delta, range) = self.segment(index)?;
        if range == 0 {
            return Ok(value.wrapping_add(delta));
        }
        let id = read::u16(self.data, self.glyph_array_offset(index, value)?)?;
        Ok(if id == 0 { 0 } else { id.wrapping_add(delta) })
    }

    fn lookup(self, value: u32) -> Result<u16, FontError> {
        match self.format {
            0 if value < 256 => Ok(u16::from(read::u8(self.data, add(6, offset(value)?)?)?)),
            4 if value <= 0xffff => {
                let i = read::lower_bound(self.count, |i| {
                    Ok(u32::from(read::u16(self.data, add(14, mul(i, 2)?)?)?) < value)
                })?;
                if i == self.count || u32::from(self.segment(i)?.0) > value {
                    return Ok(0);
                }
                self.segment_glyph(
                    i,
                    u16::try_from(value).map_err(|_| FontError::InvalidTable)?,
                )
            }
            6 => {
                let first = u32::from(read::u16(self.data, 6)?);
                let Some(i) = value.checked_sub(first) else {
                    return Ok(0);
                };
                if offset(i)? >= self.count {
                    return Ok(0);
                }
                read::u16(self.data, add(10, mul(offset(i)?, 2)?)?)
            }
            12 | 13 => {
                let i = read::lower_bound(self.count, |i| {
                    Ok(read::u32(self.data, add(20, mul(i, 12)?)?)? < value)
                })?;
                if i == self.count {
                    return Ok(0);
                }
                let group = read::bytes(self.data, add(16, mul(i, 12)?)?, 12)?;
                let start = read::u32(group, 0)?;
                if start > value {
                    return Ok(0);
                }
                let id = read::u32(group, 8)?;
                let id = if self.format == 12 {
                    id.checked_add(value.checked_sub(start).ok_or(FontError::InvalidTable)?)
                        .ok_or(FontError::Overflow)?
                } else {
                    id
                };
                u16::try_from(id).map_err(|_| FontError::GlyphIndex)
            }
            _ => Ok(0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Variations<'a> {
    data: &'a [u8],
    count: usize,
}

const fn scalar_range(start: u32, end: u32) -> bool {
    start <= end && end <= 0x0010_ffff && !(start <= 0xdfff && end >= 0xd800)
}

impl<'a> Variations<'a> {
    fn parse(data: &'a [u8], glyphs: u16) -> Result<Self, FontError> {
        let data = read::bytes(data, 0, offset(read::u32(data, 2)?)?)?;
        let count = offset(read::u32(data, 6)?)?;
        if count > 260 {
            return Err(FontError::LimitExceeded);
        }
        let end = add(10, mul(count, 11)?)?;
        let records = read::bytes(data, 10, mul(count, 11)?)?;
        let table = Self { data, count };
        let mut previous = None;
        let mut work = 0usize;
        for record in records.as_chunks::<11>().0 {
            let selector = read::u24(record, 0)?;
            if !matches!(selector, 0x180b..=0x180d | 0x180f | 0xfe00..=0xfe0f | 0xe0100..=0xe01ef)
                || previous.is_some_and(|p| p >= selector)
            {
                return Err(FontError::InvalidTable);
            }
            previous = Some(selector);
            let defaults = table.records(record, 3, 4, end)?;
            let nondefaults = table.records(record, 7, 5, end)?;
            work = add(work, add(defaults.len(), nondefaults.len())?)?;
            if work > 16 * 1024 * 1024 {
                return Err(FontError::LimitExceeded);
            }
            let mut last = None;
            for range in defaults.as_chunks::<4>().0 {
                let start = read::u24(range, 0)?;
                let end = start
                    .checked_add(u32::from(read::u8(range, 3)?))
                    .ok_or(FontError::Overflow)?;
                if !scalar_range(start, end) || last.is_some_and(|p| p >= start) {
                    return Err(FontError::InvalidTable);
                }
                last = Some(end);
            }
            let mut last = None;
            let mut ranges = defaults.as_chunks::<4>().0.iter();
            let mut range = ranges.next();
            for mapping in nondefaults.as_chunks::<5>().0 {
                let value = read::u24(mapping, 0)?;
                if !scalar_range(value, value) || last.is_some_and(|p| p >= value) {
                    return Err(FontError::InvalidTable);
                }
                last = Some(value);
                glyph(u32::from(read::u16(mapping, 3)?), glyphs)?;
                while let Some(r) = range {
                    let start = read::u24(r, 0)?;
                    let end = start
                        .checked_add(u32::from(read::u8(r, 3)?))
                        .ok_or(FontError::Overflow)?;
                    if end < value {
                        range = ranges.next();
                    } else {
                        if start <= value {
                            return Err(FontError::InvalidTable);
                        }
                        break;
                    }
                }
            }
        }
        Ok(table)
    }

    fn records(
        self,
        record: &[u8],
        field: usize,
        size: usize,
        minimum: usize,
    ) -> Result<&'a [u8], FontError> {
        let start = offset(read::u32(record, field)?)?;
        if start == 0 {
            return Ok(&[]);
        }
        if start < minimum {
            return Err(FontError::Overlap);
        }
        let count = offset(read::u32(self.data, start)?)?;
        read::bytes(self.data, add(start, 4)?, mul(count, size)?)
    }

    fn lookup(
        self,
        value: u32,
        selector: u32,
        primary: Subtable<'_>,
    ) -> Result<Option<u16>, FontError> {
        let i = read::lower_bound(self.count, |i| {
            Ok(read::u24(self.data, add(10, mul(i, 11)?)?)? < selector)
        })?;
        if i == self.count {
            return Ok(None);
        }
        let record = read::bytes(self.data, add(10, mul(i, 11)?)?, 11)?;
        if read::u24(record, 0)? != selector {
            return Ok(None);
        }
        let mappings = self.records(record, 7, 5, 0)?;
        let i = read::lower_bound(mappings.len() / 5, |i| {
            Ok(read::u24(mappings, mul(i, 5)?)? < value)
        })?;
        if i < mappings.len() / 5 && read::u24(mappings, mul(i, 5)?)? == value {
            return Ok(Some(read::u16(mappings, add(mul(i, 5)?, 3)?)?));
        }
        let ranges = self.records(record, 3, 4, 0)?;
        let i = read::lower_bound(ranges.len() / 4, |i| {
            let at = mul(i, 4)?;
            let end = read::u24(ranges, at)?
                .checked_add(u32::from(read::u8(ranges, add(at, 3)?)?))
                .ok_or(FontError::Overflow)?;
            Ok(end < value)
        })?;
        if i < ranges.len() / 4 && read::u24(ranges, mul(i, 4)?)? <= value {
            return Ok(Some(primary.lookup(value)?));
        }
        Ok(None)
    }
}
