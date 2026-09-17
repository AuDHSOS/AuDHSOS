// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{
    Fixed, FontError,
    read::{self, add, mul, sub},
};

/// A validated OpenType item variation store.
#[derive(Clone, Copy, Debug)]
pub struct ItemStore<'a> {
    data: &'a [u8],
    regions: &'a [u8],
    axes: usize,
    region_count: usize,
    count: usize,
}
impl<'a> ItemStore<'a> {
    /// Validate a store and its referenced delta arrays.
    /// # Errors
    /// Rejects malformed counts, regions, offsets, and more than 64 axes or 4096 regions/subtables.
    pub fn parse(data: &'a [u8], axes: usize) -> Result<Self, FontError> {
        if read::u16(data, 0)? != 1 {
            return Err(FontError::UnsupportedFormat);
        }
        let count = usize::from(read::u16(data, 6)?);
        read::bytes(data, 8, mul(count, 4)?)?;
        let regions = read::tail(data, read::offset(read::u32(data, 2)?)?)?;
        if usize::from(read::u16(regions, 0)?) != axes {
            return Err(FontError::InvalidTable);
        }
        let region_count = usize::from(read::u16(regions, 2)?);
        if axes > 64 || region_count > 4096 || count > 4096 {
            return Err(FontError::LimitExceeded);
        }
        let coords = read::bytes(regions, 4, mul(mul(region_count, axes)?, 6)?)?;
        for pair in coords.as_chunks::<2>().0 {
            if !(-16384..=16384).contains(&read::i16(pair, 0)?) {
                return Err(FontError::InvalidTable);
            }
        }
        let result = Self {
            data,
            regions,
            axes,
            region_count,
            count,
        };
        let mut work = 0;
        for index in 0..count {
            if let Some(row) = result.row(index)? {
                let n = usize::from(read::u16(row, 0)?);
                let flags = read::u16(row, 2)?;
                let words = usize::from(flags & 0x7fff);
                let regions = usize::from(read::u16(row, 4)?);
                if words > regions {
                    return Err(FontError::InvalidTable);
                }
                let indices = read::bytes(row, 6, mul(regions, 2)?)?;
                for pair in indices.as_chunks::<2>().0 {
                    if usize::from(read::u16(pair, 0)?) >= region_count {
                        return Err(FontError::InvalidTable);
                    }
                }
                let stride = mul(
                    add(words, regions)?,
                    if flags & 0x8000 != 0 { 2 } else { 1 },
                )?;
                let length = add(add(6, indices.len())?, mul(n, stride)?)?;
                read::bytes(row, 0, length)?;
                work = add(work, length)?;
                if work > 16 * 1024 * 1024 {
                    return Err(FontError::LimitExceeded);
                }
            }
        }
        Ok(result)
    }
    fn row(self, index: usize) -> Result<Option<&'a [u8]>, FontError> {
        if index >= self.count {
            return Err(FontError::InvalidTable);
        }
        let at = read::offset(read::u32(self.data, add(8, mul(index, 4)?)?)?)?;
        if at == 0 {
            Ok(None)
        } else {
            Ok(Some(read::tail(self.data, at)?))
        }
    }
    /// Interpolate a delta set at explicitly normalized coordinates.
    /// # Errors
    /// Returns invalid indices, coordinates, or arithmetic errors.
    pub fn delta(self, outer: u16, inner: u16, coords: &[Fixed]) -> Result<Fixed, FontError> {
        self.validate_coords(coords)?;
        if outer == 0xffff && inner == 0xffff {
            return Ok(Fixed::ZERO);
        }
        let Some(row) = self.row(usize::from(outer))? else {
            return Ok(Fixed::ZERO);
        };
        if inner >= read::u16(row, 0)? {
            return Err(FontError::InvalidTable);
        }
        let flags = read::u16(row, 2)?;
        let words = usize::from(flags & 0x7fff);
        let count = usize::from(read::u16(row, 4)?);
        let wide = flags & 0x8000 != 0;
        let stride = mul(add(words, count)?, if wide { 2 } else { 1 })?;
        let mut at = add(add(6, mul(count, 2)?)?, mul(usize::from(inner), stride)?)?;
        let mut total = Fixed::ZERO;
        for column in 0..count {
            let region = usize::from(read::u16(row, add(6, mul(column, 2)?)?)?);
            let long = column < words;
            let (value, size) = match (wide, long) {
                (true, true) => (read::i32(row, at)?, 4),
                (false, false) => (i32::from(i8::from_be_bytes([read::u8(row, at)?])), 1),
                _ => (i32::from(read::i16(row, at)?), 2),
            };
            total = total.checked_add(
                self.scalar(region, coords)?
                    .mul_ratio(i64::from(value), 1)?,
            )?;
            at = add(at, size)?;
        }
        Ok(total)
    }
    /// Validate normalized coordinates against this store's axes.
    /// # Errors
    /// Rejects mismatched counts or coordinates outside [-1, 1].
    pub fn validate_coords(self, coords: &[Fixed]) -> Result<(), FontError> {
        let min = Fixed::from_i32(-1);
        if coords.len() != self.axes || coords.iter().any(|v| *v < min || *v > Fixed::ONE) {
            return Err(FontError::InvalidTable);
        }
        Ok(())
    }
    /// Evaluate one variation region.
    /// # Errors
    /// Rejects an invalid region index, coordinates, or arithmetic overflow.
    pub fn scalar(self, region: usize, coords: &[Fixed]) -> Result<Fixed, FontError> {
        self.validate_coords(coords)?;
        if region >= self.region_count {
            return Err(FontError::InvalidTable);
        }
        let mut scalar = Fixed::ONE;
        for (axis, coordinate) in coords.iter().copied().enumerate() {
            let at = add(4, mul(add(mul(region, self.axes)?, axis)?, 6)?)?;
            let start = Fixed::from_2_14(read::i16(self.regions, at)?);
            let peak = Fixed::from_2_14(read::i16(self.regions, add(at, 2)?)?);
            let end = Fixed::from_2_14(read::i16(self.regions, add(at, 4)?)?);
            if start > peak
                || peak > end
                || (start < Fixed::ZERO && end > Fixed::ZERO)
                || peak == Fixed::ZERO
            {
                continue;
            }
            if coordinate == peak {
                continue;
            }
            if coordinate <= start || coordinate >= end {
                return Ok(Fixed::ZERO);
            }
            let factor = if coordinate < peak {
                coordinate
                    .checked_sub(start)?
                    .checked_div(peak.checked_sub(start)?)?
            } else {
                end.checked_sub(coordinate)?
                    .checked_div(end.checked_sub(peak)?)?
            };
            scalar = scalar.checked_mul(factor)?;
        }
        Ok(scalar)
    }
}

/// A packed delta-set index map, formats zero and one.
#[derive(Clone, Copy, Debug)]
pub struct DeltaMap<'a> {
    data: &'a [u8],
    count: usize,
    width: usize,
    bits: u32,
}
impl<'a> DeltaMap<'a> {
    /// Validate the packed map array.
    /// # Errors
    /// Rejects invalid formats, empty maps, excessive counts, or truncated entries.
    pub fn parse(data: &'a [u8]) -> Result<Self, FontError> {
        let format = read::u8(data, 1)?;
        if format & 0xc0 != 0 {
            return Err(FontError::InvalidTable);
        }
        let width = add(usize::from((format >> 4) & 3), 1)?;
        let bits = u32::from(format & 15)
            .checked_add(1)
            .ok_or(FontError::Overflow)?;
        let (count, at) = match read::u8(data, 0)? {
            0 => (usize::from(read::u16(data, 2)?), 4),
            1 => (read::offset(read::u32(data, 2)?)?, 6),
            _ => return Err(FontError::UnsupportedFormat),
        };
        if count == 0 {
            return Err(FontError::InvalidTable);
        }
        if count > 65536 {
            return Err(FontError::LimitExceeded);
        }
        let result = Self {
            data: read::bytes(data, at, mul(count, width)?)?,
            count,
            width,
            bits,
        };
        for index in 0..count {
            result.get(index)?;
        }
        Ok(result)
    }
    /// Map an index; indices beyond the array use its last entry.
    /// # Errors
    /// Returns an error if the packed indices cannot fit u16.
    pub fn get(self, index: usize) -> Result<(u16, u16), FontError> {
        let index = index.min(sub(self.count, 1)?);
        let mut value = 0_u32;
        for byte in read::bytes(self.data, mul(index, self.width)?, self.width)? {
            value = value
                .checked_mul(256)
                .and_then(|v| v.checked_add(u32::from(*byte)))
                .ok_or(FontError::Overflow)?;
        }
        let mask = 1_u32
            .checked_shl(self.bits)
            .and_then(|v| v.checked_sub(1))
            .ok_or(FontError::Overflow)?;
        Ok((
            u16::try_from(value >> self.bits).map_err(|_| FontError::InvalidTable)?,
            u16::try_from(value & mask).map_err(|_| FontError::InvalidTable)?,
        ))
    }
}
