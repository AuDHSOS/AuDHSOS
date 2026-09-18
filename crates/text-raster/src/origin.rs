// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R4: the fractional glyph origin of layout to a pixel and one of four
//! horizontal subpixel positions (D-174).
//!
//! This is the only place in the stack where a position becomes an integer.
//! The quantization changes no advance, no line break and no box: a glyph at a
//! subpixel position is the same outline translated.

use text_core::Fixed;

use crate::error::RasterError;

/// Horizontal positions per pixel (D-174). The number is part of the cache key
/// of D-184, so changing it invalidates every cached entry.
pub const SUBPIXEL_POSITIONS: u32 = 4;

/// One quantized glyph origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Origin {
    /// The pixel column the glyph's origin sits in.
    pub x: i32,
    /// The pixel row the glyph's origin sits on.
    pub y: i32,
    /// Which of the [`SUBPIXEL_POSITIONS`] the origin took.
    pub subpixel: u8,
}

impl Origin {
    /// The horizontal offset the subpixel position stands for, in pixels.
    #[must_use]
    pub fn offset(self) -> Fixed {
        Fixed::from_bits(
            i64::from(self.subpixel).saturating_mul(
                Fixed::ONE
                    .bits()
                    .checked_div(i64::from(SUBPIXEL_POSITIONS))
                    .unwrap_or(0),
            ),
        )
    }
}

/// Round `value` to the nearest integer, ties to even, which is the rule D-159
/// states for every precision reduction of this stack.
fn round(value: Fixed) -> Result<i64, RasterError> {
    Ok(value.mul_ratio(1, Fixed::ONE.bits())?.bits())
}

/// Quantize a fractional device-space glyph origin.
///
/// The horizontal origin takes one of four positions per pixel and the
/// vertical origin a whole pixel, which is D-174.
/// # Errors
/// Returns `Overflow` when a rounded coordinate leaves `i32`.
pub fn quantize(x: Fixed, y: Fixed) -> Result<Origin, RasterError> {
    let positions = i64::from(SUBPIXEL_POSITIONS);
    let scaled = round(x.mul_ratio(positions, 1)?)?;
    Ok(Origin {
        x: i32::try_from(scaled.div_euclid(positions)).map_err(|_| RasterError::Overflow)?,
        y: i32::try_from(round(y)?).map_err(|_| RasterError::Overflow)?,
        subpixel: u8::try_from(scaled.rem_euclid(positions)).map_err(|_| RasterError::Overflow)?,
    })
}
