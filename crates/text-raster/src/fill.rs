// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R3: edges to antialiased coverage, by exact area accumulation over a
//! scanline with the non-zero winding rule (D-182).
//!
//! Invariants: every byte written lies inside the mask, because a column is
//! below the mask's width before an offset is computed from it; no coverage
//! value exceeds `255`, because the accumulated area is clamped to one whole
//! pixel before it is scaled; malformed geometry draws rather than failing,
//! because the rule the winding number states is defined for it.

use text_core::Fixed;

use crate::{
    error::RasterError,
    flatten::Edge,
    surface::{Format, Surface},
};

/// Subpixel steps per pixel in each direction. One pixel of area is
/// `SUBPIXEL * SUBPIXEL` of these, which a `u8` of coverage is scaled from.
const SUBPIXEL: i64 = 256;

/// Q32.32 bits per subpixel step.
const BITS_PER_STEP: u32 = 32 - 8;

/// Twice the area of one whole pixel, which is what a cell accumulates.
const FULL: i64 = 2 * SUBPIXEL * SUBPIXEL;

/// The two numbers one pixel column of one scanline accumulates (D-182).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// The signed height of the edges inside this cell.
    cover: i64,
    /// Twice the signed area between the edges and the cell's right border.
    area: i64,
}

/// One point in subpixel steps.
type Step = (i64, i64);

/// Fill `mask` with the coverage of `edges`, by the non-zero winding rule.
///
/// The edges are in the mask's own pixel coordinates: `(0, 0)` is the top left
/// corner of its first pixel. Every visible pixel is written, so the caller
/// need not clear the mask first. `edges` is reordered in place.
///
/// Malformed geometry is font data and is drawn: a self-intersecting contour
/// is what the non-zero rule is defined for, a contour that does not return to
/// its start leaves the gap open, and a horizontal edge deposits nothing.
/// # Errors
/// Returns `Format` unless `mask` is [`Format::A8`], `BufferTooSmall` when
/// `cells` is shorter than the mask's width plus one, and `Overflow` when an
/// accumulator leaves `i64`.
pub fn fill(
    edges: &mut [Edge],
    mask: &mut Surface<'_>,
    cells: &mut [Cell],
) -> Result<(), RasterError> {
    if mask.format() != Format::A8 {
        return Err(RasterError::Format);
    }
    let width = usize::try_from(mask.width()).map_err(|_| RasterError::Overflow)?;
    let columns = width.checked_add(1).ok_or(RasterError::Overflow)?;
    let cells = cells
        .get_mut(..columns)
        .ok_or(RasterError::BufferTooSmall)?;
    let limit = i64::from(mask.width())
        .checked_mul(SUBPIXEL)
        .ok_or(RasterError::Overflow)?;
    edges.sort_unstable_by_key(key);
    let mut first = 0_usize;
    let mut pending = 0_usize;
    for row in 0..mask.height() {
        let top = i64::from(row)
            .checked_mul(SUBPIXEL)
            .ok_or(RasterError::Overflow)?;
        let bottom = top.checked_add(SUBPIXEL).ok_or(RasterError::Overflow)?;
        while let Some(edge) = edges.get(pending) {
            if step(edge.y0.min(edge.y1)) >= bottom {
                break;
            }
            pending = pending.checked_add(1).ok_or(RasterError::Overflow)?;
        }
        let mut index = first;
        while index < pending {
            let edge = *edges.get(index).ok_or(RasterError::Overflow)?;
            if step(edge.y0.max(edge.y1)) <= top {
                edges.swap(index, first);
                first = first.checked_add(1).ok_or(RasterError::Overflow)?;
            } else {
                deposit(&edge, top, bottom, limit, cells)?;
            }
            index = index.checked_add(1).ok_or(RasterError::Overflow)?;
        }
        sweep(mask, row, cells)?;
        for cell in cells.iter_mut() {
            *cell = Cell::default();
        }
    }
    Ok(())
}

/// The total order edges are sorted by: the top first, and then numbers of the
/// edge itself, so two edges the order cannot separate are interchangeable and
/// the result does not depend on the sort's stability.
fn key(edge: &Edge) -> (Fixed, Fixed, Fixed, Fixed) {
    (
        edge.y0.min(edge.y1),
        edge.y0.max(edge.y1),
        edge.x0.min(edge.x1),
        edge.x0.max(edge.x1),
    )
}

/// One Q32.32 coordinate in subpixel steps, rounded to nearest.
const fn step(value: Fixed) -> i64 {
    let half = 1_i64 << (BITS_PER_STEP - 1);
    value.bits().saturating_add(half) >> BITS_PER_STEP
}

/// `a + (b - a) * numerator / denominator`, rounded to nearest, ties away from
/// zero, over a wide intermediate.
fn along(a: i64, b: i64, numerator: i64, denominator: i64) -> Result<i64, RasterError> {
    if denominator == 0 {
        return Ok(a);
    }
    let span = i128::from(b)
        .checked_sub(i128::from(a))
        .ok_or(RasterError::Overflow)?;
    let scaled = span
        .checked_mul(i128::from(numerator))
        .ok_or(RasterError::Overflow)?;
    let divisor = i128::from(denominator);
    let half = divisor.abs().checked_div(2).ok_or(RasterError::Overflow)?;
    let bias = if scaled < 0 {
        half.checked_neg().ok_or(RasterError::Overflow)?
    } else {
        half
    };
    let signed = bias
        .checked_mul(divisor.signum())
        .ok_or(RasterError::Overflow)?;
    let quotient = scaled
        .checked_add(signed)
        .ok_or(RasterError::Overflow)?
        .checked_div(divisor)
        .ok_or(RasterError::Overflow)?;
    i64::try_from(
        i128::from(a)
            .checked_add(quotient)
            .ok_or(RasterError::Overflow)?,
    )
    .map_err(|_| RasterError::Overflow)
}

/// Deposit the part of `edge` inside the row `[top, bottom)` into `cells`.
fn deposit(
    edge: &Edge,
    top: i64,
    bottom: i64,
    limit: i64,
    cells: &mut [Cell],
) -> Result<(), RasterError> {
    let (x0, y0) = (step(edge.x0), step(edge.y0));
    let (x1, y1) = (step(edge.x1), step(edge.y1));
    if y0 == y1 {
        return Ok(());
    }
    let low = y0.min(y1).max(top);
    let high = y0.max(y1).min(bottom);
    if low >= high {
        return Ok(());
    }
    let span = y1.checked_sub(y0).ok_or(RasterError::Overflow)?;
    let at = |y: i64| -> Result<i64, RasterError> {
        Ok(along(
            x0,
            x1,
            y.checked_sub(y0).ok_or(RasterError::Overflow)?,
            span,
        )?
        .clamp(0, limit))
    };
    let (start, end) = if y1 > y0 {
        ((at(low)?, low), (at(high)?, high))
    } else {
        ((at(high)?, high), (at(low)?, low))
    };
    walk(start, end, cells)
}

/// Deposit one row-local segment, cell by cell.
fn walk(start: Step, end: Step, cells: &mut [Cell]) -> Result<(), RasterError> {
    let mut column = start.0.div_euclid(SUBPIXEL);
    let last = end.0.div_euclid(SUBPIXEL);
    let mut at = start;
    let forward = end.0 > start.0;
    while column != last {
        let boundary = if forward {
            column.checked_add(1).ok_or(RasterError::Overflow)?
        } else {
            column
        }
        .checked_mul(SUBPIXEL)
        .ok_or(RasterError::Overflow)?;
        let y = along(
            at.1,
            end.1,
            boundary.checked_sub(at.0).ok_or(RasterError::Overflow)?,
            end.0.checked_sub(at.0).ok_or(RasterError::Overflow)?,
        )?;
        cell(cells, column, at, (boundary, y))?;
        at = (boundary, y);
        column = if forward {
            column.checked_add(1).ok_or(RasterError::Overflow)?
        } else {
            column.checked_sub(1).ok_or(RasterError::Overflow)?
        };
    }
    cell(cells, column, at, end)
}

/// Accumulate one segment that lies inside one cell.
fn cell(cells: &mut [Cell], column: i64, from: Step, to: Step) -> Result<(), RasterError> {
    let index = usize::try_from(column).map_err(|_| RasterError::Overflow)?;
    let Some(cell) = cells.get_mut(index) else {
        return Ok(());
    };
    let height = to.1.checked_sub(from.1).ok_or(RasterError::Overflow)?;
    if height == 0 {
        return Ok(());
    }
    let left = column.checked_mul(SUBPIXEL).ok_or(RasterError::Overflow)?;
    let a = from.0.checked_sub(left).ok_or(RasterError::Overflow)?;
    let b = to.0.checked_sub(left).ok_or(RasterError::Overflow)?;
    let width = (2 * SUBPIXEL)
        .checked_sub(a)
        .ok_or(RasterError::Overflow)?
        .checked_sub(b)
        .ok_or(RasterError::Overflow)?;
    cell.cover = cell
        .cover
        .checked_add(height)
        .ok_or(RasterError::Overflow)?;
    cell.area = cell
        .area
        .checked_add(height.checked_mul(width).ok_or(RasterError::Overflow)?)
        .ok_or(RasterError::Overflow)?;
    Ok(())
}

/// Turn one row of cells into coverage bytes.
fn sweep(mask: &mut Surface<'_>, row: u32, cells: &[Cell]) -> Result<(), RasterError> {
    let mut accumulated = 0_i64;
    let target = mask.row_mut(row).ok_or(RasterError::OutOfBounds)?;
    for (byte, cell) in target.iter_mut().zip(cells) {
        let total = accumulated
            .checked_mul(2 * SUBPIXEL)
            .ok_or(RasterError::Overflow)?
            .checked_add(cell.area)
            .ok_or(RasterError::Overflow)?;
        accumulated = accumulated
            .checked_add(cell.cover)
            .ok_or(RasterError::Overflow)?;
        let magnitude = total.saturating_abs().min(FULL);
        *byte = u8::try_from(
            magnitude
                .checked_mul(255)
                .ok_or(RasterError::Overflow)?
                .checked_add(FULL / 2)
                .ok_or(RasterError::Overflow)?
                / FULL,
        )
        .map_err(|_| RasterError::Overflow)?;
    }
    Ok(())
}
