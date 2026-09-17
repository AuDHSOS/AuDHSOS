// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::FontError;

pub(crate) fn i16(data: &[u8], start: usize) -> Result<i16, FontError> {
    Ok(i16::from_be_bytes(array(data, start)?))
}

pub(crate) fn i32(data: &[u8], start: usize) -> Result<i32, FontError> {
    Ok(i32::from_be_bytes(array(data, start)?))
}

pub(crate) fn sub(a: usize, b: usize) -> Result<usize, FontError> {
    a.checked_sub(b).ok_or(FontError::InvalidTable)
}

pub(crate) fn u8(data: &[u8], start: usize) -> Result<u8, FontError> {
    data.get(start).copied().ok_or(FontError::Truncated)
}

pub(crate) fn u24(data: &[u8], start: usize) -> Result<u32, FontError> {
    let [a, b, c] = array(data, start)?;
    Ok(u32::from_be_bytes([0, a, b, c]))
}

pub(crate) fn tail(data: &[u8], start: usize) -> Result<&[u8], FontError> {
    data.get(start..).ok_or(FontError::Truncated)
}

pub(crate) fn lower_bound(
    mut count: usize,
    mut before: impl FnMut(usize) -> Result<bool, FontError>,
) -> Result<usize, FontError> {
    let mut first = 0;
    while count != 0 {
        let step = count / 2;
        let middle = add(first, step)?;
        if before(middle)? {
            first = add(middle, 1)?;
            count = sub(count, add(step, 1)?)?;
        } else {
            count = step;
        }
    }
    Ok(first)
}

pub(crate) fn add(a: usize, b: usize) -> Result<usize, FontError> {
    a.checked_add(b).ok_or(FontError::Overflow)
}

pub(crate) fn mul(a: usize, b: usize) -> Result<usize, FontError> {
    a.checked_mul(b).ok_or(FontError::Overflow)
}

pub(crate) fn offset(value: u32) -> Result<usize, FontError> {
    usize::try_from(value).map_err(|_| FontError::Overflow)
}

pub(crate) fn bytes(data: &[u8], start: usize, len: usize) -> Result<&[u8], FontError> {
    data.get(start..add(start, len)?)
        .ok_or(FontError::Truncated)
}

pub(crate) fn array<const N: usize>(data: &[u8], start: usize) -> Result<[u8; N], FontError> {
    bytes(data, start, N)?
        .try_into()
        .map_err(|_| FontError::Truncated)
}

pub(crate) fn u16(data: &[u8], start: usize) -> Result<u16, FontError> {
    Ok(u16::from_be_bytes(array(data, start)?))
}

pub(crate) fn u32(data: &[u8], start: usize) -> Result<u32, FontError> {
    Ok(u32::from_be_bytes(array(data, start)?))
}

pub(crate) const fn aligned(value: usize) -> Result<(), FontError> {
    if value.is_multiple_of(4) {
        Ok(())
    } else {
        Err(FontError::Misaligned)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl Span {
    pub(crate) fn new(data: &[u8], start: usize, len: usize) -> Result<Self, FontError> {
        bytes(data, start, len)?;
        Ok(Self {
            start,
            end: add(start, len)?,
        })
    }

    pub(crate) const fn intersects(self, other: Self) -> bool {
        self.start < self.end
            && other.start < other.end
            && self.start < other.end
            && other.start < self.end
    }

    // Empty payloads may not name a byte inside metadata either.
    pub(crate) const fn touches_metadata(self, metadata: Self) -> bool {
        self.intersects(metadata) || (self.start >= metadata.start && self.start < metadata.end)
    }
}
