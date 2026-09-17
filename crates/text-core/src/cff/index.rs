// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{
    FontError,
    read::{self, add, mul},
};

/// A borrowed, fully bounds-checked CFF or CFF2 INDEX.
#[derive(Clone, Copy, Debug)]
pub struct Index<'a> {
    offsets: &'a [u8],
    data: &'a [u8],
    width: usize,
    count: usize,
}

impl<'a> Index<'a> {
    pub(super) const EMPTY: Self = Self {
        offsets: &[],
        data: &[],
        width: 0,
        count: 0,
    };
    /// Parse an INDEX and return its total encoded length.
    /// # Errors
    /// Rejects invalid widths, counts above 65536, nonmonotonic offsets, or truncation.
    pub fn parse(bytes: &'a [u8], cff2: bool) -> Result<(Self, usize), FontError> {
        Self::read(bytes, cff2, true)
    }

    pub(super) fn validated(bytes: &'a [u8], cff2: bool) -> Result<Self, FontError> {
        Ok(Self::read(bytes, cff2, false)?.0)
    }

    fn read(bytes: &'a [u8], cff2: bool, validate: bool) -> Result<(Self, usize), FontError> {
        let (count, header) = if cff2 {
            (read::offset(read::u32(bytes, 0)?)?, 4)
        } else {
            (usize::from(read::u16(bytes, 0)?), 2)
        };
        if count == 0 {
            return Ok((Self::EMPTY, header));
        }
        if count > 65536 {
            return Err(FontError::LimitExceeded);
        }
        let width = usize::from(read::u8(bytes, header)?);
        if !(1..=4).contains(&width) {
            return Err(FontError::InvalidTable);
        }
        let offsets = read::bytes(bytes, add(header, 1)?, mul(add(count, 1)?, width)?)?;
        let mut result = Self {
            offsets,
            data: &[],
            width,
            count,
        };
        if result.offset(0)? != 1 {
            return Err(FontError::InvalidTable);
        }
        let mut previous = result.offset(count)?;
        if validate {
            previous = 1;
            for i in 1..=count {
                let next = result.offset(i)?;
                if next < previous {
                    return Err(FontError::InvalidTable);
                }
                previous = next;
            }
        }
        let start = add(add(header, 1)?, offsets.len())?;
        result.data = read::bytes(bytes, start, read::sub(previous, 1)?)?;
        Ok((result, add(start, result.data.len())?))
    }
    fn offset(self, index: usize) -> Result<usize, FontError> {
        let mut value = 0_usize;
        for b in read::bytes(self.offsets, mul(index, self.width)?, self.width)? {
            value = add(mul(value, 256)?, usize::from(*b))?;
        }
        Ok(value)
    }
    /// Number of objects in the INDEX.
    #[must_use]
    pub const fn len(self) -> usize {
        self.count
    }
    /// Whether this INDEX contains no objects.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }
    /// Return one object by its zero-based index.
    /// # Errors
    /// Returns `GlyphIndex` when the index exceeds the object count.
    pub fn get(self, index: usize) -> Result<&'a [u8], FontError> {
        if index >= self.count {
            return Err(FontError::GlyphIndex);
        }
        let start = read::sub(self.offset(index)?, 1)?;
        let end = read::sub(self.offset(add(index, 1)?)?, 1)?;
        read::bytes(self.data, start, read::sub(end, start)?)
    }
}
