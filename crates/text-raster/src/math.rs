// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R8: the transcendentals the flattening and the colour path need, in
//! `Fixed` Q32.32 integers and to a stated error bound.
//!
//! `text_core::colr::trig` gives sine and cosine in half-turns to within
//! `2^-28`. A sweep gradient needs the inverse, a radial gradient and the
//! segment count of D-181 need a square root, and the gamma tables of R5 need
//! a power. D-177 admits no floating point, so each is an integer series after
//! an exact reduction.

use text_core::Fixed;

use crate::error::RasterError;

/// One over pi, rounded to nearest in Q32.32. Turns radians into half-turns.
const INV_PI: Fixed = Fixed::from_bits(1_367_130_551);
/// The natural logarithm of two, rounded to nearest in Q32.32.
const LN2: Fixed = Fixed::from_bits(2_977_044_472);
/// `tan(pi/8)`, the tangent at which the octant reduction of `atan` switches.
const TAN_PI_8: Fixed = Fixed::from_bits(1_779_033_704);
/// An eighth of a turn, 45 degrees, as 0.25 half-turns: the offset the folded
/// branch of `atan` adds.
const EIGHTH_TURN: Fixed = Fixed::from_bits(1 << 30);
/// A quarter of a turn, 90 degrees, as 0.5 half-turns: the angle the two
/// arguments are exchanged about.
const QUARTER_TURN: Fixed = Fixed::from_bits(1 << 31);
/// Terms of the odd Taylor series of `atan`, through `w^27`.
const ATAN_TERMS: i64 = 14;
/// Terms of the Taylor series of `exp`, through `t^12`.
const EXP_TERMS: i64 = 12;
/// Fractional bits the binary logarithm extracts.
const LOG2_BITS: u32 = 32;
/// The largest gamma value [`crate::Gamma`] accepts, and the largest exponent
/// [`pow`] is written for.
pub const MAX_EXPONENT: Fixed = Fixed::from_bits(16 << 32);

/// The square root of `value`, correctly rounded.
///
/// The result `r` is whichever of the two neighbouring Q32.32 numbers is
/// nearer the real root; no argument is exactly halfway, because `(r + 1/2)^2`
/// is never an integer.
/// # Errors
/// Returns `Domain` for a negative argument.
pub fn sqrt(value: Fixed) -> Result<Fixed, RasterError> {
    let bits = value.bits();
    if bits < 0 {
        return Err(RasterError::Domain);
    }
    // `bits` is below `2^63` and the shift keeps it below `2^95`.
    let wide = u128::from(bits.unsigned_abs())
        .checked_shl(32)
        .ok_or(RasterError::Overflow)?;
    let root = wide.isqrt();
    let square = root.checked_mul(root).ok_or(RasterError::Overflow)?;
    let rest = wide.checked_sub(square).ok_or(RasterError::Overflow)?;
    let root = if rest > root {
        root.checked_add(1).ok_or(RasterError::Overflow)?
    } else {
        root
    };
    i64::try_from(root)
        .map(Fixed::from_bits)
        .map_err(|_| RasterError::Overflow)
}

/// The angle of the direction `(x, y)` in half-turns, in `(-1, 1]`.
///
/// One half-turn is 180 degrees, which is the unit COLR states a sweep
/// gradient's angles in (`docs/microsoft/colr.html:2745`). The result is within
/// `2^-28` of the real angle. `atan2_turns(0, 0)` is zero.
/// # Errors
/// Returns `Overflow` when an intermediate exceeds Q32.32.
pub fn atan2_turns(y: Fixed, x: Fixed) -> Result<Fixed, RasterError> {
    if x == Fixed::ZERO && y == Fixed::ZERO {
        return Ok(Fixed::ZERO);
    }
    let ax = absolute(x)?;
    let ay = absolute(y)?;
    let angle = if ay <= ax {
        atan_unit(ay.checked_div(ax)?)?
    } else {
        QUARTER_TURN.checked_sub(atan_unit(ax.checked_div(ay)?)?)?
    };
    let angle = if x.bits() < 0 {
        Fixed::ONE.checked_sub(angle)?
    } else {
        angle
    };
    if y.bits() < 0 {
        Ok(angle.checked_neg()?)
    } else {
        Ok(angle)
    }
}

/// `base` raised to `exponent`, within `2^-24` of the real value.
///
/// Evaluated as `exp2(exponent * log2(base))`. `pow(0, e)` is zero.
/// # Errors
/// Returns `Domain` unless `base` is in `[0, 1]` and `exponent` in
/// `(0, MAX_EXPONENT]`.
pub fn pow(base: Fixed, exponent: Fixed) -> Result<Fixed, RasterError> {
    if base.bits() < 0 || base > Fixed::ONE || exponent <= Fixed::ZERO || exponent > MAX_EXPONENT {
        return Err(RasterError::Domain);
    }
    if base == Fixed::ZERO {
        return Ok(Fixed::ZERO);
    }
    exp2_negative(exponent.checked_mul(log2_unit(base)?)?)
}

/// `|value|`.
fn absolute(value: Fixed) -> Result<Fixed, RasterError> {
    if value.bits() < 0 {
        Ok(value.checked_neg()?)
    } else {
        Ok(value)
    }
}

/// `atan(ratio) / pi` for `ratio` in `[0, 1]`, a value in `[0, 1/4]`.
///
/// Above `tan(pi/8)` the identity `atan(z) = pi/4 + atan((z - 1) / (z + 1))`
/// moves the argument into `[-tan(pi/8), 0]`, so the series is evaluated on an
/// argument of magnitude at most `tan(pi/8)` either way.
fn atan_unit(ratio: Fixed) -> Result<Fixed, RasterError> {
    if ratio > TAN_PI_8 {
        let folded = ratio
            .checked_sub(Fixed::ONE)?
            .checked_div(ratio.checked_add(Fixed::ONE)?)?;
        EIGHTH_TURN.checked_add(atan_series(folded)?.checked_mul(INV_PI)?)
    } else {
        atan_series(ratio)?.checked_mul(INV_PI)
    }
    .map_err(RasterError::from)
}

/// `atan(w)` in radians for `|w| <= tan(pi/8)`, by the odd Taylor series
/// through `w^27`, evaluated on `w^2` by Horner.
fn atan_series(w: Fixed) -> Result<Fixed, RasterError> {
    let square = w.checked_mul(w)?;
    let mut divisor = 2_i64.saturating_mul(ATAN_TERMS).saturating_sub(1);
    let mut sum = Fixed::ONE.mul_ratio(1, divisor)?;
    while divisor > 1 {
        divisor = divisor.saturating_sub(2);
        sum = Fixed::ONE
            .mul_ratio(1, divisor)?
            .checked_sub(square.checked_mul(sum)?)?;
    }
    Ok(w.checked_mul(sum)?)
}

/// `log2(value)` for `value` in `(0, 1]`, a value in `(-32, 0]`.
///
/// The integer part is the normalizing shift; each fractional bit is one
/// squaring of the mantissa, which stays in `[1, 2)`.
fn log2_unit(value: Fixed) -> Result<Fixed, RasterError> {
    let mut mantissa = value;
    let mut shift: i32 = 0;
    while mantissa < Fixed::ONE {
        mantissa = mantissa.mul_ratio(2, 1)?;
        shift = shift.checked_add(1).ok_or(RasterError::Overflow)?;
    }
    let mut fraction: i64 = 0;
    for bit in 1..=LOG2_BITS {
        mantissa = mantissa.checked_mul(mantissa)?;
        if mantissa >= Fixed::from_i32(2) {
            mantissa = mantissa.mul_ratio(1, 2)?;
            fraction |= 1_i64
                .checked_shl(LOG2_BITS.checked_sub(bit).ok_or(RasterError::Overflow)?)
                .ok_or(RasterError::Overflow)?;
        }
    }
    Fixed::from_bits(fraction)
        .checked_sub(Fixed::from_i32(shift))
        .map_err(RasterError::from)
}

/// `2^value` for `value <= 0`.
///
/// `value` is split into `-(whole + fraction)` and the result is
/// `2^(1 - fraction)` shifted right by `whole + 1`, so the series runs on
/// `(1 - fraction) * ln 2`, which is in `(0, ln 2]`.
fn exp2_negative(value: Fixed) -> Result<Fixed, RasterError> {
    if value > Fixed::ZERO {
        return Err(RasterError::Domain);
    }
    let magnitude = value.checked_neg()?.bits();
    // A shift of 64 or more leaves nothing of the mantissa.
    let Ok(whole) = u32::try_from(magnitude >> 32) else {
        return Ok(Fixed::ZERO);
    };
    let Some(shift) = whole.checked_add(1).filter(|shift| *shift < 63) else {
        return Ok(Fixed::ZERO);
    };
    let fraction = Fixed::from_bits(magnitude & ((1_i64 << 32) - 1));
    let t = Fixed::ONE.checked_sub(fraction)?.checked_mul(LN2)?;
    let mut sum = Fixed::ONE;
    for term in (1..=EXP_TERMS).rev() {
        sum = Fixed::ONE.checked_add(t.checked_mul(sum)?.mul_ratio(1, term)?)?;
    }
    sum.mul_ratio(1, 1_i64.checked_shl(shift).ok_or(RasterError::Overflow)?)
        .map_err(RasterError::from)
}
