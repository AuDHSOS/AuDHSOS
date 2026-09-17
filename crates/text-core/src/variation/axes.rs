// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{
    Fixed, FontError,
    read::{self, add, mul},
};

/// One user-space variation axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axis {
    /// Four-byte OpenType axis tag.
    pub tag: [u8; 4],
    /// Minimum coordinate.
    pub min: Fixed,
    /// Default coordinate.
    pub default: Fixed,
    /// Maximum coordinate.
    pub max: Fixed,
}
/// An explicitly requested user-space coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisValue {
    /// Four-byte axis tag.
    pub tag: [u8; 4],
    /// User-space coordinate, clamped to the axis range.
    pub value: Fixed,
}

/// Validated fvar axes with optional avar segment maps.
#[derive(Clone, Copy, Debug)]
pub struct Axes<'a> {
    data: &'a [u8],
    count: usize,
    stride: usize,
    avar: Option<&'a [u8]>,
}
impl<'a> Axes<'a> {
    /// Parse fvar and optional avar version one.
    /// # Errors
    /// Rejects invalid records, duplicate tags, malformed segments, or over 64 axes.
    pub fn parse(fvar: &'a [u8], avar: Option<&'a [u8]>) -> Result<Self, FontError> {
        if read::u32(fvar, 0)? != 0x0001_0000 || read::u16(fvar, 6)? != 2 {
            return Err(FontError::UnsupportedFormat);
        }
        let at = usize::from(read::u16(fvar, 4)?);
        let count = usize::from(read::u16(fvar, 8)?);
        let stride = usize::from(read::u16(fvar, 10)?);
        if at < 16 || stride < 20 {
            return Err(FontError::InvalidTable);
        }
        if count > 64 {
            return Err(FontError::LimitExceeded);
        }
        let data = read::bytes(fvar, at, mul(count, stride)?)?;
        let result = Self {
            data,
            count,
            stride,
            avar,
        };
        for index in 0..count {
            let axis = result.axis(index)?;
            if axis.min > axis.default
                || axis.default > axis.max
                || !axis.tag.iter().all(|c| (32..=126).contains(c))
            {
                return Err(FontError::InvalidTable);
            }
            for prior in 0..index {
                if result.axis(prior)?.tag == axis.tag {
                    return Err(FontError::InvalidTable);
                }
            }
        }
        let instances = usize::from(read::u16(fvar, 12)?);
        let size = usize::from(read::u16(fvar, 14)?);
        let minimum = add(mul(count, 4)?, 4)?;
        if size != minimum && size != add(minimum, 2)? {
            return Err(FontError::InvalidTable);
        }
        let records = read::bytes(fvar, add(at, data.len())?, mul(instances, size)?)?;
        for index in 0..instances {
            let record = read::bytes(records, mul(index, size)?, size)?;
            if read::u16(record, 2)? != 0 {
                return Err(FontError::InvalidTable);
            }
            for axis in 0..count {
                let value = Fixed::from_16_16(read::i32(record, add(4, mul(axis, 4)?)?)?);
                let bounds = result.axis(axis)?;
                if value < bounds.min || value > bounds.max {
                    return Err(FontError::InvalidTable);
                }
            }
        }
        if let Some(avar) = avar {
            if read::u32(avar, 0)? != 0x0001_0000
                || read::u16(avar, 4)? != 0
                || usize::from(read::u16(avar, 6)?) != count
            {
                return Err(FontError::InvalidTable);
            }
            let mut at = 8;
            for _ in 0..count {
                let n = usize::from(read::u16(avar, at)?);
                let pairs = read::bytes(avar, add(at, 2)?, mul(n, 4)?)?;
                validate_map(pairs)?;
                at = add(add(at, 2)?, pairs.len())?;
            }
        }
        Ok(result)
    }
    /// Number of axes in source order.
    #[must_use]
    pub const fn len(self) -> usize {
        self.count
    }
    /// Whether the font has no functional variation axes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }
    /// Read one axis by index.
    /// # Errors
    /// Returns `InvalidTable` for an invalid index or record.
    pub fn axis(self, index: usize) -> Result<Axis, FontError> {
        if index >= self.count {
            return Err(FontError::InvalidTable);
        }
        let record = read::bytes(self.data, mul(index, self.stride)?, 20)?;
        if read::u16(record, 16)? & !1 != 0 {
            return Err(FontError::InvalidTable);
        }
        Ok(Axis {
            tag: read::array(record, 0)?,
            min: Fixed::from_16_16(read::i32(record, 4)?),
            default: Fixed::from_16_16(read::i32(record, 8)?),
            max: Fixed::from_16_16(read::i32(record, 12)?),
        })
    }
    /// Normalize requests into caller storage in fvar axis order.
    ///
    /// Unknown or repeated request tags are rejected; omitted axes use defaults.
    /// # Errors
    /// Returns invalid-axis, arithmetic, or caller-buffer errors.
    pub fn normalize(self, values: &[AxisValue], out: &mut [Fixed]) -> Result<usize, FontError> {
        let out = out.get_mut(..self.count).ok_or(FontError::BufferTooSmall)?;
        for (i, value) in values.iter().enumerate() {
            if values
                .get(..i)
                .ok_or(FontError::InvalidTable)?
                .iter()
                .any(|v| v.tag == value.tag)
            {
                return Err(FontError::InvalidTable);
            }
            let mut found = false;
            for index in 0..self.count {
                found |= self.axis(index)?.tag == value.tag;
            }
            if !found {
                return Err(FontError::InvalidTable);
            }
        }
        let mut map_at = 8;
        for (index, target) in out.iter_mut().enumerate() {
            let axis = self.axis(index)?;
            let value = values
                .iter()
                .find(|v| v.tag == axis.tag)
                .map_or(axis.default, |v| v.value)
                .clamp(axis.min, axis.max);
            *target = match value.cmp(&axis.default) {
                core::cmp::Ordering::Equal => Fixed::ZERO,
                core::cmp::Ordering::Less => value
                    .checked_sub(axis.default)?
                    .checked_div(axis.default.checked_sub(axis.min)?)?,
                core::cmp::Ordering::Greater => value
                    .checked_sub(axis.default)?
                    .checked_div(axis.max.checked_sub(axis.default)?)?,
            };
            if let Some(avar) = self.avar {
                let n = usize::from(read::u16(avar, map_at)?);
                let pairs = read::bytes(avar, add(map_at, 2)?, mul(n, 4)?)?;
                *target = map(*target, pairs)?;
                map_at = add(add(map_at, 2)?, pairs.len())?;
            }
        }
        Ok(self.count)
    }
}

fn validate_map(data: &[u8]) -> Result<(), FontError> {
    let mut previous = None;
    for pair in data.as_chunks::<4>().0 {
        let from = read::i16(pair, 0)?;
        let to = read::i16(pair, 2)?;
        if !(-16384..=16384).contains(&from) || !(-16384..=16384).contains(&to) {
            return Err(FontError::InvalidTable);
        }
        if previous.is_some_and(|(a, b)| from <= a || to < b) {
            return Err(FontError::InvalidTable);
        }
        previous = Some((from, to));
    }
    Ok(())
}
fn map(value: Fixed, data: &[u8]) -> Result<Fixed, FontError> {
    let mut anchors = 0;
    for pair in data.as_chunks::<4>().0 {
        let from = read::i16(pair, 0)?;
        let to = read::i16(pair, 2)?;
        if from == to && matches!(from, -16384 | 0 | 16384) {
            anchors = add(anchors, 1)?;
        }
    }
    if anchors != 3 {
        return Ok(value);
    }
    let mut previous = (Fixed::ZERO, Fixed::ZERO);
    for pair in data.as_chunks::<4>().0 {
        let from = Fixed::from_2_14(read::i16(pair, 0)?);
        let to = Fixed::from_2_14(read::i16(pair, 2)?);
        if value == from {
            return Ok(to);
        }
        if value < from {
            return previous.1.checked_add(
                value
                    .checked_sub(previous.0)?
                    .checked_mul(to.checked_sub(previous.1)?)?
                    .checked_div(from.checked_sub(previous.0)?)?,
            );
        }
        previous = (from, to);
    }
    Ok(value)
}
