// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Sine, cosine and tangent of an angle in half-turns, in Q32.32 integers.
//!
//! COLR states rotation and skew as an angle where 1.0 means 180 degrees
//! (`docs/microsoft/colr.html:2745`, `docs/microsoft/colr.html:2910`). D2
//! admits no floating point, so the three functions are integer Taylor series
//! after reduction to a quarter turn.

use crate::{Fixed, FontError};

/// Pi, rounded to nearest in Q32.32.
const PI: Fixed = Fixed::from_bits(13_493_037_705);
/// A quarter turn, 0.5 half-turns, in Q32.32 bits.
const QUARTER: i64 = 1_i64 << 31;
/// An eighth of a turn, 0.25 half-turns, in Q32.32 bits.
const EIGHTH: i64 = 1_i64 << 30;
/// One full turn less one bit: the mask of a two-half-turn reduction.
const TURN_MASK: i64 = (1_i64 << 33) - 1;

/// Sine and cosine of `turns` half-turns.
///
/// The argument is reduced modulo two half-turns exactly, then to `[0, pi/4]`
/// by quadrant symmetry. The truncated tail of each series and the rounding of
/// the eleven operations that evaluate it keep the result within `2^-28` of
/// the real value, sixteen Q32.32 units; a dense sweep of both functions found
/// no error above nine units.
/// # Errors
/// Returns `Overflow` when an intermediate exceeds Q32.32.
pub(crate) fn sin_cos(turns: Fixed) -> Result<(Fixed, Fixed), FontError> {
    // A power-of-two modulus: the low bits of a two's-complement i64 are the
    // Euclidean remainder, so the reduction is exact for either sign.
    let unit = turns.bits() & TURN_MASK;
    let quadrant = unit >> 31;
    let rest = unit & (QUARTER - 1);
    let (angle, swapped) = if rest > EIGHTH {
        (QUARTER.wrapping_sub(rest), true)
    } else {
        (rest, false)
    };
    let radians = Fixed::from_bits(angle).checked_mul(PI)?;
    let (sine, cosine) = if swapped {
        (cos_series(radians)?, sin_series(radians)?)
    } else {
        (sin_series(radians)?, cos_series(radians)?)
    };
    Ok(match quadrant {
        0 => (sine, cosine),
        1 => (cosine, sine.checked_neg()?),
        2 => (sine.checked_neg()?, cosine.checked_neg()?),
        _ => (cosine.checked_neg()?, sine),
    })
}

/// Tangent of `turns` half-turns.
/// # Errors
/// Returns `InvalidTable` at an odd quarter turn, where the tangent has a pole.
pub(crate) fn tan(turns: Fixed) -> Result<Fixed, FontError> {
    let (sine, cosine) = sin_cos(turns)?;
    sine.checked_div(cosine)
}

/// `sin(x)` for `x` in `[0, pi/4]`, Taylor through the ninth power.
fn sin_series(x: Fixed) -> Result<Fixed, FontError> {
    let square = x.checked_mul(x)?;
    let mut term = Fixed::ONE;
    for divisor in [72, 42, 20, 6] {
        term = Fixed::ONE.checked_sub(square.checked_mul(term)?.mul_ratio(1, divisor)?)?;
    }
    x.checked_mul(term)
}

/// `cos(x)` for `x` in `[0, pi/4]`, Taylor through the tenth power.
fn cos_series(x: Fixed) -> Result<Fixed, FontError> {
    let square = x.checked_mul(x)?;
    let mut term = Fixed::ONE;
    for divisor in [90, 56, 30, 12, 2] {
        term = Fixed::ONE.checked_sub(square.checked_mul(term)?.mul_ratio(1, divisor)?)?;
    }
    Ok(term)
}
