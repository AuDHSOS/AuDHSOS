// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{B, TextError, Unit, add, isolate, removed};

/// Number of visual indices after X9 controls have been excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineInfo {
    /// Populated prefix of the order buffer; indices refer to the supplied units.
    pub count: usize,
}
/// Apply L1/L2 to one line without changing paragraph resolution.
///
/// Supply a scalar-index range contained in one paragraph. `levels` needs one
/// entry per line scalar (255 denotes X9 controls); order needs the same capacity.
/// # Errors
/// Returns invalid range/paragraph, capacity, or arithmetic errors.
pub fn reorder_line(
    units: &[Unit],
    range: core::ops::Range<usize>,
    levels: &mut [u8],
    order: &mut [usize],
) -> Result<LineInfo, TextError> {
    let line = units.get(range.clone()).ok_or(TextError::InvalidInput)?;
    let levels = levels
        .get_mut(..line.len())
        .ok_or(TextError::BufferTooSmall)?;
    let order = order
        .get_mut(..line.len())
        .ok_or(TextError::BufferTooSmall)?;
    let base = line.first().map_or(0, |u| u.base);
    if line.iter().any(|u| u.base != base)
        || line
            .get(..line.len().saturating_sub(1))
            .is_some_and(|s| s.iter().any(|u| u.original == B::B))
    {
        return Err(TextError::InvalidInput);
    }
    for (out, u) in levels.iter_mut().zip(line) {
        *out = u.level;
    }
    // L1: retain X9 controls as transparent during whitespace resets.
    let mut trailing_start = 0;
    for (i, u) in line.iter().enumerate() {
        if matches!(u.original, B::B | B::S) {
            levels
                .get_mut(trailing_start..=i)
                .ok_or(TextError::Overflow)?
                .fill(base);
            trailing_start = add(i, 1)?;
        } else if !matches!(u.original, B::Ws | B::Pdi)
            && !isolate(u.original)
            && !removed(u.original)
        {
            trailing_start = add(i, 1)?;
        }
    }
    levels
        .get_mut(trailing_start..)
        .ok_or(TextError::Overflow)?
        .fill(base);
    let mut count = 0;
    let mut maximum = 0;
    let mut minimum_odd = 127;
    for (i, (level, u)) in levels.iter_mut().zip(line).enumerate() {
        if removed(u.original) {
            *level = 255;
            continue;
        }
        maximum = maximum.max(*level);
        if *level & 1 != 0 {
            minimum_odd = minimum_odd.min(*level);
        }
        *order.get_mut(count).ok_or(TextError::BufferTooSmall)? = add(range.start, i)?;
        count = add(count, 1)?;
    }
    let order = order.get_mut(..count).ok_or(TextError::BufferTooSmall)?;
    for level in (minimum_odd..=maximum).rev() {
        let mut start = 0;
        for i in 0..=count {
            let in_run = order
                .get(i)
                .and_then(|index| index.checked_sub(range.start))
                .and_then(|local| levels.get(local))
                .is_some_and(|v| *v >= level);
            if !in_run {
                order
                    .get_mut(start..i)
                    .ok_or(TextError::Overflow)?
                    .reverse();
                start = add(i, 1)?;
            }
        }
    }
    Ok(LineInfo { count })
}
