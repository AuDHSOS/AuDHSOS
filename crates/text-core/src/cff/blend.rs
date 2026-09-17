// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::dict::offset;
use crate::{
    Fixed, FontError,
    read::{self, add, mul, sub},
};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Blend<'a> {
    pub store: Option<Store<'a>>,
    pub coords: &'a [Fixed],
    pub index: usize,
}
impl Blend<'_> {
    pub(super) fn select(&mut self, index: usize) -> Result<(), FontError> {
        self.store
            .ok_or(FontError::MissingTable)?
            .validate_index(index)?;
        self.index = index;
        Ok(())
    }
    pub(super) fn apply(self, stack: &mut [Fixed], len: &mut usize) -> Result<(), FontError> {
        self.store
            .ok_or(FontError::MissingTable)?
            .blend(stack, len, self.index, self.coords)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Store<'a> {
    data: &'a [u8],
    regions: &'a [u8],
    axes: usize,
    count: usize,
}
impl<'a> Store<'a> {
    pub(super) fn parse(data: &'a [u8]) -> Result<Self, FontError> {
        let len = usize::from(read::u16(data, 0)?);
        let data = if len == 65535 {
            read::tail(data, 2)?
        } else {
            read::bytes(data, 2, len)?
        };
        if read::u16(data, 0)? != 1 {
            return Err(FontError::UnsupportedFormat);
        }
        let count = usize::from(read::u16(data, 6)?);
        read::bytes(data, 8, mul(count, 4)?)?;
        let regions = read::tail(data, read::offset(read::u32(data, 2)?)?)?;
        let axes = usize::from(read::u16(regions, 0)?);
        let region_count = usize::from(read::u16(regions, 2)?);
        if axes > 64 || region_count > 4096 || count > 4096 {
            return Err(FontError::LimitExceeded);
        }
        let coordinates = read::bytes(regions, 4, mul(mul(axes, region_count)?, 6)?)?;
        for pair in coordinates.as_chunks::<2>().0 {
            if !(-16384..=16384).contains(&read::i16(pair, 0)?) {
                return Err(FontError::InvalidTable);
            }
        }
        let result = Self {
            data,
            regions,
            axes,
            count,
        };
        let mut validation_bytes = 0;
        for index in 0..count {
            let row = result.row(index)?;
            if read::u16(row, 0)? != 0 || read::u16(row, 2)? != 0 {
                return Err(FontError::InvalidTable);
            }
            let count = usize::from(read::u16(row, 4)?);
            validation_bytes = add(validation_bytes, mul(count, 2)?)?;
            if validation_bytes > 16_777_216 {
                return Err(FontError::LimitExceeded);
            }
            for index in read::bytes(row, 6, mul(count, 2)?)?.as_chunks::<2>().0 {
                if usize::from(read::u16(index, 0)?) >= region_count {
                    return Err(FontError::InvalidTable);
                }
            }
        }
        Ok(result)
    }
    fn row(self, index: usize) -> Result<&'a [u8], FontError> {
        if index >= self.count {
            return Err(FontError::InvalidTable);
        }
        read::tail(
            self.data,
            read::offset(read::u32(self.data, add(8, mul(index, 4)?)?)?)?,
        )
    }
    pub(super) fn validate_index(self, index: usize) -> Result<(), FontError> {
        self.row(index)?;
        Ok(())
    }
    pub(super) fn blend(
        self,
        stack: &mut [Fixed],
        len: &mut usize,
        index: usize,
        coords: &[Fixed],
    ) -> Result<(), FontError> {
        *len = sub(*len, 1)?;
        let n = offset(*stack.get(*len).ok_or(FontError::InvalidTable)?)?;
        if n == 0 {
            return Err(FontError::InvalidTable);
        }
        if !coords.is_empty() && coords.len() != self.axes {
            return Err(FontError::InvalidTable);
        }
        let row = self.row(index)?;
        let k = usize::from(read::u16(row, 4)?);
        let first = sub(*len, mul(n, add(k, 1)?)?)?;
        for i in 0..n {
            let at = add(first, i)?;
            let mut value = *stack.get(at).ok_or(FontError::InvalidTable)?;
            for region in 0..k {
                let id = usize::from(read::u16(row, add(6, mul(region, 2)?)?)?);
                let scalar = self.scalar(id, coords)?;
                let delta_at = add(add(first, n)?, add(mul(i, k)?, region)?)?;
                value = value.checked_add(
                    stack
                        .get(delta_at)
                        .ok_or(FontError::InvalidTable)?
                        .checked_mul(scalar)?,
                )?;
            }
            *stack.get_mut(at).ok_or(FontError::InvalidTable)? = value;
        }
        *len = add(first, n)?;
        Ok(())
    }
    pub(super) fn validate_coords(self, coords: &[Fixed]) -> Result<(), FontError> {
        if coords.len() != self.axes
            || coords
                .iter()
                .any(|v| *v < Fixed::from_i32(-1) || *v > Fixed::ONE)
        {
            return Err(FontError::InvalidTable);
        }
        Ok(())
    }
    fn scalar(self, region: usize, coords: &[Fixed]) -> Result<Fixed, FontError> {
        let mut scalar = Fixed::ONE;
        for axis in 0..self.axes {
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
            let coordinate = coords.get(axis).copied().unwrap_or(Fixed::ZERO);
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
