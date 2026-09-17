// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{DeltaMap, ItemStore};
use crate::{
    Fixed, FontError,
    read::{self, add, mul},
};

/// Horizontal metric variation deltas; apply once to hmtx metrics.
#[derive(Clone, Copy, Debug)]
pub struct Hvar<'a> {
    store: ItemStore<'a>,
    advance: Option<DeltaMap<'a>>,
    left: Option<DeltaMap<'a>>,
    right: Option<DeltaMap<'a>>,
}
impl<'a> Hvar<'a> {
    /// Parse HVAR with the fvar axis count.
    /// # Errors
    /// Rejects invalid versions, offsets, stores, or maps.
    pub fn parse(data: &'a [u8], axes: usize) -> Result<Self, FontError> {
        if read::u32(data, 0)? != 0x0001_0000 {
            return Err(FontError::UnsupportedFormat);
        }
        let at = read::offset(read::u32(data, 4)?)?;
        if at < 20 {
            return Err(FontError::InvalidTable);
        }
        let store = ItemStore::parse(read::tail(data, at)?, axes)?;
        let map = |offset| -> Result<Option<DeltaMap<'a>>, FontError> {
            let at = read::offset(read::u32(data, offset)?)?;
            if at == 0 {
                Ok(None)
            } else if at < 20 {
                Err(FontError::InvalidTable)
            } else {
                Ok(Some(DeltaMap::parse(read::tail(data, at)?)?))
            }
        };
        Ok(Self {
            store,
            advance: map(8)?,
            left: map(12)?,
            right: map(16)?,
        })
    }
    /// Interpolated advance delta in design units.
    /// # Errors
    /// Rejects invalid delta indices, coordinates, or arithmetic overflow.
    pub fn advance(self, glyph: u16, coords: &[Fixed]) -> Result<Fixed, FontError> {
        let (outer, inner) = self
            .advance
            .map_or(Ok((0, glyph)), |m| m.get(usize::from(glyph)))?;
        self.store.delta(outer, inner, coords)
    }
    /// Interpolated side-bearing delta, absent when no map is supplied.
    /// # Errors
    /// Rejects invalid delta indices, coordinates, or arithmetic overflow.
    pub fn side_bearing(
        self,
        glyph: u16,
        left: bool,
        coords: &[Fixed],
    ) -> Result<Option<Fixed>, FontError> {
        let map = if left { self.left } else { self.right };
        map.map(|m| {
            let (outer, inner) = m.get(usize::from(glyph))?;
            self.store.delta(outer, inner, coords)
        })
        .transpose()
    }
}

/// Font-wide metric variation records, keyed by OpenType metric tags.
#[derive(Clone, Copy, Debug)]
pub struct Mvar<'a> {
    records: &'a [u8],
    stride: usize,
    count: usize,
    store: Option<ItemStore<'a>>,
}
impl<'a> Mvar<'a> {
    /// Parse MVAR with the fvar axis count.
    /// # Errors
    /// Rejects malformed headers, unordered tags, or invalid store data.
    pub fn parse(data: &'a [u8], axes: usize) -> Result<Self, FontError> {
        if read::u32(data, 0)? != 0x0001_0000 || read::u16(data, 4)? != 0 {
            return Err(FontError::UnsupportedFormat);
        }
        let stride = usize::from(read::u16(data, 6)?);
        let count = usize::from(read::u16(data, 8)?);
        let at = usize::from(read::u16(data, 10)?);
        if stride < 8 {
            return Err(FontError::InvalidTable);
        }
        let records = read::bytes(data, 12, mul(count, stride)?)?;
        if (count == 0) != (at == 0) || (count != 0 && at < add(12, records.len())?) {
            return Err(FontError::InvalidTable);
        }
        let store = if at == 0 {
            None
        } else {
            Some(ItemStore::parse(read::tail(data, at)?, axes)?)
        };
        let mut previous = None;
        for i in 0..count {
            let tag = read::u32(records, mul(i, stride)?)?;
            if previous.is_some_and(|p| p >= tag) {
                return Err(FontError::TableOrder);
            }
            previous = Some(tag);
        }
        Ok(Self {
            records,
            stride,
            count,
            store,
        })
    }
    /// Interpolate a metric adjustment; an absent tag has zero delta.
    /// # Errors
    /// Rejects invalid delta-set indices, coordinates, or arithmetic overflow.
    pub fn delta(self, tag: [u8; 4], coords: &[Fixed]) -> Result<Fixed, FontError> {
        let i = read::lower_bound(self.count, |i| {
            Ok(read::array::<4>(self.records, mul(i, self.stride)?)? < tag)
        })?;
        if i == self.count {
            return Ok(Fixed::ZERO);
        }
        let at = mul(i, self.stride)?;
        if read::array::<4>(self.records, at)? != tag {
            return Ok(Fixed::ZERO);
        }
        self.store.ok_or(FontError::MissingTable)?.delta(
            read::u16(self.records, add(at, 4)?)?,
            read::u16(self.records, add(at, 6)?)?,
            coords,
        )
    }
}
