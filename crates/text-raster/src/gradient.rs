// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R9: the three gradient shapes and the three extend modes, evaluated in the
//! fill's own coordinate space.
//!
//! A `PaintOp::Fill` carries the transform from the fill's space to font
//! space. Inverting it, composed with the glyph's own transform, maps a device
//! pixel back into the space the gradient is stated in, which is the only
//! place a radial gradient's circles are circles and a sweep gradient's angles
//! are angles.
//!
//! Every colour is interpolated on premultiplied linear-light values, which
//! `docs/microsoft/cpal.html:985` requires.

use text_core::{
    Fixed,
    colr::{ColorLine, ColorStop, Extend, Fill},
};

use crate::{
    error::RasterError,
    gamma::Gamma,
    math::{atan2_turns, sqrt},
    pixel::{Paint, Pixel},
    transform::{Affine, Invertible},
};

/// The largest magnitude a normalized gradient coordinate may take, so that a
/// product of two of them and a sum of two such products stay inside Q32.32.
const NORMAL: i64 = 128;

/// A full turn in half-turns.
const TURN: Fixed = Fixed::from_bits(2 << 32);

/// One gradient, its geometry normalized and its inverse transform taken once.
#[derive(Clone, Copy, Debug)]
pub struct Gradient<'a> {
    shape: Shape,
    line: ColorLine,
    stops: &'a [ColorStop],
    inverse: Affine,
    shift: u32,
}

/// The normalized geometry of one gradient shape.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// Start point and the p3 direction the specification constructs.
    Linear {
        x0: Fixed,
        y0: Fixed,
        ux: Fixed,
        uy: Fixed,
    },
    /// Two circles.
    Radial {
        x0: Fixed,
        y0: Fixed,
        r0: Fixed,
        dx: Fixed,
        dy: Fixed,
        dr: Fixed,
    },
    /// A centre and two angles in half-turns, which no scale changes.
    Sweep {
        x: Fixed,
        y: Fixed,
        start: Fixed,
        end: Fixed,
    },
}

impl<'a> Gradient<'a> {
    /// Prepare one gradient for evaluation at device pixels.
    ///
    /// `placement` maps the fill's own space to device space: the glyph's
    /// transform composed with the one the `PaintOp::Fill` carries.
    /// Returns `None` for a gradient the specification does not define, which
    /// draws nothing: a singular placement, a linear gradient whose p3 falls
    /// on p0, and a colour line without stops.
    /// # Errors
    /// Returns `Overflow` when an intermediate exceeds Q32.32.
    pub fn new(
        fill: Fill,
        stops: &'a [ColorStop],
        placement: Affine,
    ) -> Result<Option<Self>, RasterError> {
        let Some(inverse) = placement.inverse()? else {
            return Ok(None);
        };
        let (shape, line, shift) = match fill {
            Fill::Solid(_) => return Ok(None),
            Fill::Linear {
                x0,
                y0,
                x1,
                y1,
                x2,
                y2,
                line,
            } => {
                let shift = shift_for(&[x0, y0, x1, y1, x2, y2]);
                let (x0, y0) = (narrow(x0, shift)?, narrow(y0, shift)?);
                let along = (
                    narrow(x1, shift)?.checked_sub(x0)?,
                    narrow(y1, shift)?.checked_sub(y0)?,
                );
                let rotation = (
                    narrow(x2, shift)?.checked_sub(x0)?,
                    narrow(y2, shift)?.checked_sub(y0)?,
                );
                let Some((ux, uy)) = perpendicular_part(along, rotation)? else {
                    return Ok(None);
                };
                (Shape::Linear { x0, y0, ux, uy }, line, shift)
            }
            Fill::Radial {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                line,
            } => {
                let shift = shift_for(&[x0, y0, r0, x1, y1, r1]);
                let (x0, y0, r0) = (narrow(x0, shift)?, narrow(y0, shift)?, narrow(r0, shift)?);
                (
                    Shape::Radial {
                        x0,
                        y0,
                        r0,
                        dx: narrow(x1, shift)?.checked_sub(x0)?,
                        dy: narrow(y1, shift)?.checked_sub(y0)?,
                        dr: narrow(r1, shift)?.checked_sub(r0)?,
                    },
                    line,
                    shift,
                )
            }
            Fill::Sweep {
                x,
                y,
                start,
                end,
                line,
            } => {
                let shift = shift_for(&[x, y]);
                (
                    Shape::Sweep {
                        x: narrow(x, shift)?,
                        y: narrow(y, shift)?,
                        start,
                        end,
                    },
                    line,
                    shift,
                )
            }
        };
        if line.count == 0
            || stops
                .get(line.first..)
                .is_none_or(|rest| rest.len() < line.count)
        {
            return Ok(None);
        }
        Ok(Some(Self {
            shape,
            line,
            stops,
            inverse,
            shift,
        }))
    }

    /// The colour at one device pixel's centre, premultiplied and linear.
    ///
    /// Returns `None` where the shape leaves the gradient undefined, which is
    /// the cone of a radial gradient no circle of the family reaches.
    /// # Errors
    /// Returns `Overflow` when an intermediate exceeds Q32.32.
    pub fn at(
        &self,
        x: Fixed,
        y: Fixed,
        foreground: Paint,
        gamma: &Gamma,
    ) -> Result<Option<Pixel>, RasterError> {
        let (fx, fy) = self.inverse.apply(x, y)?;
        let Some(parameter) = self.parameter(narrow(fx, self.shift)?, narrow(fy, self.shift)?)?
        else {
            return Ok(None);
        };
        self.sample(parameter, foreground, gamma).map(Some)
    }

    /// The gradient parameter at one point of the fill's own space.
    fn parameter(&self, px: Fixed, py: Fixed) -> Result<Option<Fixed>, RasterError> {
        match self.shape {
            Shape::Linear { x0, y0, ux, uy } => {
                let along = dot(px.checked_sub(x0)?, py.checked_sub(y0)?, ux, uy)?;
                let square = dot(ux, uy, ux, uy)?;
                Ok(Some(divide(along, square)))
            }
            Shape::Radial {
                x0,
                y0,
                r0,
                dx,
                dy,
                dr,
            } => two_circles(px.checked_sub(x0)?, py.checked_sub(y0)?, r0, dx, dy, dr),
            Shape::Sweep { x, y, start, end } => {
                let angle = atan2_turns(py.checked_sub(y)?, px.checked_sub(x)?)?;
                let angle = if angle.bits() < 0 {
                    angle.checked_add(TURN)?
                } else {
                    angle
                };
                let span = end.checked_sub(start)?;
                if span == Fixed::ZERO {
                    return Ok(Some(Fixed::ZERO));
                }
                Ok(Some(divide(angle.checked_sub(start)?, span)))
            }
        }
    }

    /// The colour of the line at one parameter, under the extend mode.
    fn sample(
        &self,
        parameter: Fixed,
        foreground: Paint,
        gamma: &Gamma,
    ) -> Result<Pixel, RasterError> {
        let first = self.stop(0)?;
        let last = self.stop(self.line.count.saturating_sub(1))?;
        let span = last.offset.checked_sub(first.offset)?;
        if span <= Fixed::ZERO {
            // Every stop sits at one offset: the line is a step, which pads to
            // the first colour below it and the last colour at and above it.
            let stop = if parameter < first.offset {
                first
            } else {
                last
            };
            return colour(stop, foreground, gamma);
        }
        let position = divide(parameter.checked_sub(first.offset)?, span);
        let Some(position) = extend(position, self.line.extend)? else {
            return colour(first, foreground, gamma);
        };
        let at = first.offset.checked_add(position.checked_mul(span)?)?;
        let index = self.bracket(at)?;
        let low = self.stop(index)?;
        let high = self.stop(index.saturating_add(1))?;
        let width = high.offset.checked_sub(low.offset)?;
        if width <= Fixed::ZERO {
            return colour(high, foreground, gamma);
        }
        let weight = divide(at.checked_sub(low.offset)?, width).clamp(Fixed::ZERO, Fixed::ONE);
        mix(
            colour(low, foreground, gamma)?,
            colour(high, foreground, gamma)?,
            weight,
        )
    }

    /// The last stop whose offset is at or below `at`, as an index into the
    /// line, by binary search over the stops the line names.
    fn bracket(&self, at: Fixed) -> Result<usize, RasterError> {
        let mut low = 0_usize;
        let mut high = self.line.count.saturating_sub(1);
        while low < high {
            let middle = low
                .checked_add(high)
                .and_then(|sum| sum.checked_add(1))
                .ok_or(RasterError::Overflow)?
                / 2;
            if self.stop(middle)?.offset <= at {
                low = middle;
            } else {
                high = middle.saturating_sub(1);
            }
        }
        Ok(low.min(self.line.count.saturating_sub(2)))
    }

    /// One stop of this line, clamped to its last.
    fn stop(&self, index: usize) -> Result<ColorStop, RasterError> {
        let last = self.line.count.saturating_sub(1);
        let at = self
            .line
            .first
            .checked_add(index.min(last))
            .ok_or(RasterError::Overflow)?;
        self.stops.get(at).copied().ok_or(RasterError::Overflow)
    }
}

/// One stop's colour as a premultiplied linear-light pixel.
fn colour(stop: ColorStop, foreground: Paint, gamma: &Gamma) -> Result<Pixel, RasterError> {
    Paint::of(stop.color.source, stop.color.alpha, foreground).pixel(255, gamma)
}

/// `low` and `high` mixed by `weight`, on premultiplied linear-light values.
fn mix(low: Pixel, high: Pixel, weight: Fixed) -> Result<Pixel, RasterError> {
    let blend = |a: u32, b: u32| -> Result<u32, RasterError> {
        let span = i64::from(b)
            .checked_sub(i64::from(a))
            .ok_or(RasterError::Overflow)?;
        let moved = Fixed::from_bits(span.checked_mul(1 << 32).ok_or(RasterError::Overflow)?)
            .checked_mul(weight)?;
        let value = i64::from(a)
            .checked_add(moved.bits() >> 32)
            .ok_or(RasterError::Overflow)?;
        u32::try_from(value.clamp(0, i64::from(crate::gamma::LINEAR_ONE)))
            .map_err(|_| RasterError::Overflow)
    };
    Ok(Pixel {
        red: blend(low.red, high.red)?,
        green: blend(low.green, high.green)?,
        blue: blend(low.blue, high.blue)?,
        alpha: blend(low.alpha, high.alpha)?,
    })
}

/// A position on the colour line, folded into `[0, 1]` by the extend mode.
/// `None` means the position is outside a line the mode cannot extend.
fn extend(position: Fixed, mode: Extend) -> Result<Option<Fixed>, RasterError> {
    Ok(Some(match mode {
        Extend::Pad => position.clamp(Fixed::ZERO, Fixed::ONE),
        Extend::Repeat => fraction(position),
        Extend::Reflect => {
            let folded = fraction(position.mul_ratio(1, 2)?).mul_ratio(2, 1)?;
            if folded > Fixed::ONE {
                Fixed::from_i32(2).checked_sub(folded)?
            } else {
                folded
            }
        }
    }))
}

/// `value - floor(value)`, which is in `[0, 1)` for either sign.
const fn fraction(value: Fixed) -> Fixed {
    // The low bits of a two's-complement integer are the Euclidean remainder,
    // so the fractional part is exact for either sign.
    Fixed::from_bits(value.bits() & ((1_i64 << 32) - 1))
}

/// `a / b`, saturating rather than failing, because a gradient parameter far
/// outside the line is what an extend mode is for.
fn divide(numerator: Fixed, denominator: Fixed) -> Fixed {
    numerator.checked_div(denominator).unwrap_or(
        if (numerator.bits() < 0) == (denominator.bits() < 0) {
            Fixed::from_bits(i64::MAX)
        } else {
            Fixed::from_bits(i64::MIN)
        },
    )
}

/// The dot product of two vectors.
fn dot(ax: Fixed, ay: Fixed, bx: Fixed, by: Fixed) -> Result<Fixed, RasterError> {
    Ok(ax.checked_mul(bx)?.checked_add(ay.checked_mul(by)?)?)
}

/// The part of `along` perpendicular to `rotation`, which is the p3 the
/// specification constructs: the projection of p0p1 onto the line through p0
/// perpendicular to p0p2. `None` when that projection lands on p0, which
/// leaves the gradient undefined.
fn perpendicular_part(
    along: (Fixed, Fixed),
    rotation: (Fixed, Fixed),
) -> Result<Option<(Fixed, Fixed)>, RasterError> {
    let normal = (rotation.1.checked_neg()?, rotation.0);
    let square = dot(normal.0, normal.1, normal.0, normal.1)?;
    if square == Fixed::ZERO {
        // p2 sits on p0: no rotation is stated, so p3 is p1 itself.
        return Ok((along != (Fixed::ZERO, Fixed::ZERO)).then_some(along));
    }
    let scale = divide(dot(along.0, along.1, normal.0, normal.1)?, square);
    let part = (normal.0.checked_mul(scale)?, normal.1.checked_mul(scale)?);
    Ok((part != (Fixed::ZERO, Fixed::ZERO)).then_some(part))
}

/// The gradient parameter of the two-circle form: the larger root whose
/// interpolated radius is not negative.
fn two_circles(
    px: Fixed,
    py: Fixed,
    r0: Fixed,
    dx: Fixed,
    dy: Fixed,
    dr: Fixed,
) -> Result<Option<Fixed>, RasterError> {
    let quadratic = dot(dx, dy, dx, dy)?.checked_sub(dr.checked_mul(dr)?)?;
    let linear = dot(px, py, dx, dy)?.checked_add(r0.checked_mul(dr)?)?;
    let constant = dot(px, py, px, py)?.checked_sub(r0.checked_mul(r0)?)?;
    if quadratic == Fixed::ZERO {
        if linear == Fixed::ZERO {
            return Ok(None);
        }
        let root = divide(constant, linear.mul_ratio(2, 1)?);
        return Ok(admissible(root, r0, dr).then_some(root));
    }
    let discriminant = linear
        .checked_mul(linear)?
        .checked_sub(quadratic.checked_mul(constant)?)?;
    if discriminant < Fixed::ZERO {
        return Ok(None);
    }
    let spread = sqrt(discriminant)?;
    let high = divide(linear.checked_add(spread)?, quadratic);
    let low = divide(linear.checked_sub(spread)?, quadratic);
    let (first, second) = if high >= low {
        (high, low)
    } else {
        (low, high)
    };
    if admissible(first, r0, dr) {
        return Ok(Some(first));
    }
    Ok(admissible(second, r0, dr).then_some(second))
}

/// Whether the circle at this parameter has a radius that is not negative.
fn admissible(parameter: Fixed, r0: Fixed, dr: Fixed) -> bool {
    parameter
        .checked_mul(dr)
        .and_then(|moved| r0.checked_add(moved))
        .is_ok_and(|radius| radius >= Fixed::ZERO)
}

/// How far to shift a set of coordinates right so that every one of them is at
/// most [`NORMAL`]. The parameter of every shape here is invariant under one
/// scale applied to all of its inputs.
fn shift_for(values: &[Fixed]) -> u32 {
    let mut largest = 0_i64;
    for value in values {
        largest = largest.max(value.bits().saturating_abs());
    }
    let mut limit = NORMAL.saturating_mul(Fixed::ONE.bits());
    let mut shift = 0_u32;
    while shift < 32 && largest > limit {
        // The limit is what `largest` is compared against before the shift, so
        // it doubles with every bit the shift takes off.
        limit = limit.saturating_mul(2);
        shift = shift.saturating_add(1);
    }
    shift
}

/// One coordinate divided by two to the `shift`, rounded to nearest.
fn narrow(value: Fixed, shift: u32) -> Result<Fixed, RasterError> {
    if shift == 0 {
        return Ok(value);
    }
    Ok(value.mul_ratio(1, 1_i64.checked_shl(shift).ok_or(RasterError::Overflow)?)?)
}
