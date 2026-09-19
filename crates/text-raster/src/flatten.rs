// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R2: outlines to polylines in device space, within the tolerance of D-181.
//!
//! Invariants: every curve is replaced by line segments whose distance from
//! the curve is at most [`TOLERANCE`] in device space, unless the segment
//! count reached [`MAX_SEGMENTS`]; the control points are transformed before
//! the count is computed, so the count follows the size a glyph is drawn at.

use text_core::{Fixed, cff::Command, colr::Affine, glyf::Point};

use crate::{error::RasterError, math::sqrt};

/// The largest distance in device space between a curve and the polyline that
/// replaces it: one eighth of a pixel (D-181).
pub const TOLERANCE: Fixed = Fixed::from_bits(1 << 29);

/// The largest number of line segments one curve is replaced by (D-181). A
/// curve beyond it is flattened at the clamp and exceeds the tolerance.
pub const MAX_SEGMENTS: u32 = 256;

/// One line segment of a flattened contour, in device space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edge {
    /// Where the segment starts, horizontally.
    pub x0: Fixed,
    /// Where the segment starts, vertically.
    pub y0: Fixed,
    /// Where the segment ends, horizontally.
    pub x1: Fixed,
    /// Where the segment ends, vertically.
    pub y1: Fixed,
}

/// The segment count for a quadratic whose second difference has length `d`.
///
/// The deviation of a curve from its chord over a parameter interval of length
/// `h` is at most `h^2 * max|B''| / 8`, and `|B''|` is `2 * d` for a
/// quadratic, so `n = ceil(sqrt(d / (4 * TOLERANCE)))`.
#[must_use]
pub fn quadratic_segments(second_difference: Fixed) -> u32 {
    segments(second_difference, 1)
}

/// The segment count for a cubic whose largest second difference is `d`.
///
/// `|B''|` is at most `6 * d` for a cubic, so
/// `n = ceil(sqrt(3 * d / (4 * TOLERANCE)))`.
#[must_use]
pub fn cubic_segments(second_difference: Fixed) -> u32 {
    segments(second_difference, 3)
}

/// `ceil(sqrt(numerator * d / (4 * TOLERANCE)))`, at least one and at most
/// [`MAX_SEGMENTS`]. A second difference so large that the quotient or the
/// root leaves Q32.32 takes the clamp, which is what D-181 says it does.
fn segments(second_difference: Fixed, numerator: i64) -> u32 {
    if second_difference <= Fixed::ZERO {
        return 1;
    }
    let quotient = second_difference
        .mul_ratio(numerator, 1)
        .and_then(|value| value.checked_div(TOLERANCE.mul_ratio(4, 1)?));
    let Ok(root) = quotient.map_err(RasterError::from).and_then(sqrt) else {
        return MAX_SEGMENTS;
    };
    let whole = root.bits() >> 32;
    let count = if root.bits() & FRACTION == 0 {
        whole
    } else {
        whole.saturating_add(1)
    };
    u32::try_from(count.clamp(1, i64::from(MAX_SEGMENTS))).unwrap_or(MAX_SEGMENTS)
}

/// The fractional bits of a Q32.32 number.
const FRACTION: i64 = (1_i64 << 32) - 1;

/// Collects edges into the caller's slice, closing each contour.
struct Sink<'a> {
    edges: &'a mut [Edge],
    count: usize,
    start: Option<(Fixed, Fixed)>,
    at: (Fixed, Fixed),
}

impl Sink<'_> {
    fn move_to(&mut self, point: (Fixed, Fixed)) -> Result<(), RasterError> {
        self.close()?;
        self.start = Some(point);
        self.at = point;
        Ok(())
    }

    /// A segment whose endpoints coincide is dropped: it encloses no area and
    /// deposits no coverage, so keeping it would only cost the caller a slot.
    fn line_to(&mut self, point: (Fixed, Fixed)) -> Result<(), RasterError> {
        if point == self.at {
            return Ok(());
        }
        if self.start.is_none() {
            self.start = Some(self.at);
        }
        *self
            .edges
            .get_mut(self.count)
            .ok_or(RasterError::BufferTooSmall)? = Edge {
            x0: self.at.0,
            y0: self.at.1,
            x1: point.0,
            y1: point.1,
        };
        self.count = self.count.checked_add(1).ok_or(RasterError::Overflow)?;
        self.at = point;
        Ok(())
    }

    fn close(&mut self) -> Result<(), RasterError> {
        let Some(start) = self.start.take() else {
            return Ok(());
        };
        if start != self.at {
            self.start = Some(start);
            self.line_to(start)?;
            self.start = None;
        }
        Ok(())
    }

    /// One quadratic segment, its control points already in device space.
    fn quadratic(
        &mut self,
        control: (Fixed, Fixed),
        end: (Fixed, Fixed),
    ) -> Result<(), RasterError> {
        let start = self.at;
        let count = quadratic_segments(length(difference(start, control, end)?));
        for step in 1..=count {
            let t = Fixed::ONE.mul_ratio(i64::from(step), i64::from(count))?;
            let u = Fixed::ONE.checked_sub(t)?;
            let point = (
                blend_quadratic(start.0, control.0, end.0, u, t)?,
                blend_quadratic(start.1, control.1, end.1, u, t)?,
            );
            self.line_to(point)?;
        }
        Ok(())
    }

    /// One cubic segment, its control points already in device space.
    fn cubic(
        &mut self,
        first: (Fixed, Fixed),
        second: (Fixed, Fixed),
        end: (Fixed, Fixed),
    ) -> Result<(), RasterError> {
        let start = self.at;
        let count = cubic_segments(second_difference(start, first, second, end)?);
        for step in 1..=count {
            let t = Fixed::ONE.mul_ratio(i64::from(step), i64::from(count))?;
            let u = Fixed::ONE.checked_sub(t)?;
            let point = (
                blend_cubic(start.0, first.0, second.0, end.0, u, t)?,
                blend_cubic(start.1, first.1, second.1, end.1, u, t)?,
            );
            self.line_to(point)?;
        }
        Ok(())
    }
}

/// The larger of the two second differences `p0 - 2*p1 + p2` and
/// `p1 - 2*p2 + p3` of a cubic, as a length. The one second difference of a
/// quadratic is `difference(start, control, end)`.
fn second_difference(
    p0: (Fixed, Fixed),
    p1: (Fixed, Fixed),
    p2: (Fixed, Fixed),
    p3: (Fixed, Fixed),
) -> Result<Fixed, RasterError> {
    Ok(length(difference(p0, p1, p2)?).max(length(difference(p1, p2, p3)?)))
}

/// `a - 2*b + c`, per component.
fn difference(
    a: (Fixed, Fixed),
    b: (Fixed, Fixed),
    c: (Fixed, Fixed),
) -> Result<(Fixed, Fixed), RasterError> {
    Ok((
        a.0.checked_sub(b.0)?.checked_sub(b.0)?.checked_add(c.0)?,
        a.1.checked_sub(b.1)?.checked_sub(b.1)?.checked_add(c.1)?,
    ))
}

/// The Euclidean length of a vector, saturating rather than failing.
///
/// A vector whose square leaves Q32.32 is longer than 46340 device pixels,
/// which no glyph of a display reaches; D-181 says such a segment takes the
/// clamp, and the largest representable length is what makes `segments` take
/// it.
fn length(vector: (Fixed, Fixed)) -> Fixed {
    let square = vector
        .0
        .checked_mul(vector.0)
        .and_then(|first| first.checked_add(vector.1.checked_mul(vector.1)?));
    match square.map_err(RasterError::from).and_then(sqrt) {
        Ok(value) => value,
        Err(_) => Fixed::from_bits(i64::MAX),
    }
}

/// The Bernstein combination `rest^2*start + 2*rest*step*control + step^2*end`
/// of one coordinate of a quadratic.
fn blend_quadratic(
    start: Fixed,
    control: Fixed,
    end: Fixed,
    rest: Fixed,
    step: Fixed,
) -> Result<Fixed, RasterError> {
    let opening = rest.checked_mul(rest)?.checked_mul(start)?;
    let middle = rest
        .checked_mul(step)?
        .checked_mul(control)?
        .mul_ratio(2, 1)?;
    let closing = step.checked_mul(step)?.checked_mul(end)?;
    Ok(opening.checked_add(middle)?.checked_add(closing)?)
}

/// The Bernstein combination of one coordinate of a cubic.
fn blend_cubic(
    start: Fixed,
    first: Fixed,
    second: Fixed,
    end: Fixed,
    rest: Fixed,
    step: Fixed,
) -> Result<Fixed, RasterError> {
    let rest_square = rest.checked_mul(rest)?;
    let step_square = step.checked_mul(step)?;
    let opening = rest_square.checked_mul(rest)?.checked_mul(start)?;
    let early = rest_square
        .checked_mul(step)?
        .checked_mul(first)?
        .mul_ratio(3, 1)?;
    let late = rest
        .checked_mul(step_square)?
        .checked_mul(second)?
        .mul_ratio(3, 1)?;
    let closing = step_square.checked_mul(step)?.checked_mul(end)?;
    Ok(opening
        .checked_add(early)?
        .checked_add(late)?
        .checked_add(closing)?)
}

/// The midpoint of two points.
fn midpoint(a: (Fixed, Fixed), b: (Fixed, Fixed)) -> Result<(Fixed, Fixed), RasterError> {
    Ok((
        a.0.checked_add(b.0)?.mul_ratio(1, 2)?,
        a.1.checked_add(b.1)?.mul_ratio(1, 2)?,
    ))
}

/// Map one outline point into device space.
fn place(transform: Affine, x: Fixed, y: Fixed) -> Result<(Fixed, Fixed), RasterError> {
    Ok(transform.apply(x, y)?)
}

/// Flatten a TrueType outline, whose curve segments are quadratic.
///
/// `contours` holds the inclusive index of each contour's last point, which is
/// what `Glyf::outline` writes. `transform` maps font units to device pixels.
/// # Errors
/// Returns `BufferTooSmall` when `edges` cannot hold the result, `Overflow`
/// when a coordinate leaves Q32.32, and `Font` for contour ends that do not
/// describe a partition of the points.
pub fn flatten_glyf(
    points: &[Point],
    contours: &[usize],
    transform: Affine,
    edges: &mut [Edge],
) -> Result<usize, RasterError> {
    let mut sink = Sink {
        edges,
        count: 0,
        start: None,
        at: (Fixed::ZERO, Fixed::ZERO),
    };
    let mut first = 0_usize;
    for end in contours {
        let last = end.checked_add(1).ok_or(RasterError::Overflow)?;
        if last <= first || last > points.len() {
            return Err(RasterError::Font(text_core::FontError::InvalidTable));
        }
        let contour = points.get(first..last).ok_or(RasterError::Overflow)?;
        contour_glyf(&mut sink, contour, transform)?;
        first = last;
    }
    sink.close()?;
    Ok(sink.count)
}

/// One TrueType contour.
///
/// The start is the first on-curve point; without one it is the midpoint of
/// the last and first points, which is the implied on-curve point the format
/// describes at `docs/microsoft/glyf.html`.
fn contour_glyf(
    sink: &mut Sink<'_>,
    contour: &[Point],
    transform: Affine,
) -> Result<(), RasterError> {
    let count = contour.len();
    let Some(first) = contour.first() else {
        return Ok(());
    };
    let at = |index: usize| -> Option<&Point> { contour.get(index.checked_rem(count)?) };
    let (begin, mut index) = if let Some(position) = contour.iter().position(|point| point.on_curve)
    {
        let point = at(position).ok_or(RasterError::Overflow)?;
        (
            place(transform, point.x, point.y)?,
            position.checked_add(1).ok_or(RasterError::Overflow)?,
        )
    } else {
        let last = contour.last().unwrap_or(first);
        let ending = place(transform, last.x, last.y)?;
        let opening = place(transform, first.x, first.y)?;
        (midpoint(ending, opening)?, 0)
    };
    sink.move_to(begin)?;
    let mut step = 0_usize;
    while step < count {
        let point = at(index).ok_or(RasterError::Overflow)?;
        let here = place(transform, point.x, point.y)?;
        if point.on_curve {
            sink.line_to(here)?;
            index = index.checked_add(1).ok_or(RasterError::Overflow)?;
            step = step.checked_add(1).ok_or(RasterError::Overflow)?;
            continue;
        }
        let next =
            at(index.checked_add(1).ok_or(RasterError::Overflow)?).ok_or(RasterError::Overflow)?;
        let target = place(transform, next.x, next.y)?;
        if next.on_curve {
            sink.quadratic(here, target)?;
            index = index.checked_add(2).ok_or(RasterError::Overflow)?;
            step = step.checked_add(2).ok_or(RasterError::Overflow)?;
        } else {
            sink.quadratic(here, midpoint(here, target)?)?;
            index = index.checked_add(1).ok_or(RasterError::Overflow)?;
            step = step.checked_add(1).ok_or(RasterError::Overflow)?;
        }
    }
    sink.close()
}

/// Flatten a CFF or CFF2 outline, whose curve segments are cubic.
///
/// `transform` maps font units to device pixels.
/// # Errors
/// Returns `BufferTooSmall` when `edges` cannot hold the result and `Overflow`
/// when a coordinate leaves Q32.32.
pub fn flatten_cff(
    commands: &[Command],
    transform: Affine,
    edges: &mut [Edge],
) -> Result<usize, RasterError> {
    let mut sink = Sink {
        edges,
        count: 0,
        start: None,
        at: (Fixed::ZERO, Fixed::ZERO),
    };
    for command in commands {
        match *command {
            Command::Move(to) => sink.move_to(place(transform, to.x, to.y)?)?,
            Command::Line(to) => sink.line_to(place(transform, to.x, to.y)?)?,
            Command::Curve(first, second, to) => sink.cubic(
                place(transform, first.x, first.y)?,
                place(transform, second.x, second.y)?,
                place(transform, to.x, to.y)?,
            )?,
            Command::Close => sink.close()?,
        }
    }
    sink.close()?;
    Ok(sink.count)
}
