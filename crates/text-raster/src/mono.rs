// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R6: one coverage mask composited with one text colour.
//!
//! The blend runs on linear-light values and the coverage becomes alpha
//! unchanged, which is D-183. A mask pixel outside the destination is skipped,
//! so a glyph may overhang any edge.

use crate::{
    error::RasterError,
    gamma::Gamma,
    pixel::Paint,
    surface::{Mask, Surface},
};

/// Draw `mask` onto `destination` in one colour, source-over.
///
/// `at` is where the mask's top left corner lands in the destination, in
/// pixels, and may be negative or beyond the destination.
/// # Errors
/// Returns `Overflow` when a coordinate leaves `i32`.
pub fn blend_mono(
    destination: &mut Surface<'_>,
    at: (i32, i32),
    mask: Mask<'_>,
    paint: Paint,
    gamma: &Gamma,
) -> Result<(), RasterError> {
    for row in 0..mask.height() {
        let Some(y) = offset(at.1, row, destination.height()) else {
            continue;
        };
        for column in 0..mask.width() {
            let Some(x) = offset(at.0, column, destination.width()) else {
                continue;
            };
            let coverage = mask.coverage(column, row).unwrap_or(0);
            if coverage == 0 {
                continue;
            }
            let source = paint.pixel(coverage, gamma)?;
            let backdrop = destination.pixel(x, y, gamma).unwrap_or_default();
            destination.set_pixel(x, y, source.over(backdrop), gamma)?;
        }
    }
    Ok(())
}

/// `base + step`, or `None` outside `0..limit`.
fn offset(base: i32, step: u32, limit: u32) -> Option<u32> {
    let sum = base.checked_add(i32::try_from(step).ok()?)?;
    let position = u32::try_from(sum).ok()?;
    (position < limit).then_some(position)
}
