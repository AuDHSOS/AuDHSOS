// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The transform an element sits under.
//!
//! Six numbers, the ones SVG itself writes: `a` and `d` scale, `b` and `c`
//! shear, `e` and `f` move. The four that scale are thousandths — a
//! thousand is one — and the two that move are thousandths of a user unit,
//! like every other length here.
//!
//! A rotation needs no angle. Where one is asked for by name the sine and
//! the cosine are the two numbers a quarter turn apart in a table of
//! sixteenths, and where one is needed to point a marker along a line the
//! direction of the line *is* the rotation: a unit vector `(dx, dy)` and
//! the matrix `[dx dy -dy dx]` are the same thing. So there is no
//! trigonometry in this crate, and no floating-point number either.

use crate::Unit;
use crate::number::{self, ONE};

/// A transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Matrix {
    /// The x of the x axis.
    a: Unit,
    /// The y of the x axis.
    b: Unit,
    /// The x of the y axis.
    c: Unit,
    /// The y of the y axis.
    d: Unit,
    /// Where the origin moves to, across.
    e: Unit,
    /// Where the origin moves to, down.
    f: Unit,
}

impl Matrix {
    /// The transform that changes nothing.
    pub(crate) const IDENTITY: Self = Self {
        a: ONE,
        b: 0,
        c: 0,
        d: ONE,
        e: 0,
        f: 0,
    };

    /// A transform from its six numbers.
    #[expect(
        clippy::many_single_char_names,
        reason = "the six are the six of the SVG transform, and they have no other names"
    )]
    pub(crate) const fn new(a: Unit, b: Unit, c: Unit, d: Unit, e: Unit, f: Unit) -> Self {
        Self { a, b, c, d, e, f }
    }

    /// A move.
    pub(crate) const fn translation(x: Unit, y: Unit) -> Self {
        Self::new(ONE, 0, 0, ONE, x, y)
    }

    /// A scale.
    pub(crate) const fn scaling(x: Unit, y: Unit) -> Self {
        Self::new(x, 0, 0, y, 0, 0)
    }

    /// `self`, then `other`: a point goes through `self` first.
    pub(crate) const fn then(self, other: Self) -> Self {
        Self {
            a: mul(self.a, other.a).saturating_add(mul(self.b, other.c)),
            b: mul(self.a, other.b).saturating_add(mul(self.b, other.d)),
            c: mul(self.c, other.a).saturating_add(mul(self.d, other.c)),
            d: mul(self.c, other.b).saturating_add(mul(self.d, other.d)),
            e: mul(self.e, other.a)
                .saturating_add(mul(self.f, other.c))
                .saturating_add(other.e),
            f: mul(self.e, other.b)
                .saturating_add(mul(self.f, other.d))
                .saturating_add(other.f),
        }
    }

    /// Where a point goes.
    pub(crate) const fn apply(self, x: Unit, y: Unit) -> (Unit, Unit) {
        (
            mul(x, self.a)
                .saturating_add(mul(y, self.c))
                .saturating_add(self.e),
            mul(x, self.b)
                .saturating_add(mul(y, self.d))
                .saturating_add(self.f),
        )
    }

    /// How much the transform scales a length, whichever way it points:
    /// the square root of the area it multiplies by.
    pub(crate) fn scale(self) -> Unit {
        let determinant = mul(self.a, self.d).saturating_sub(mul(self.b, self.c));
        let magnitude = determinant.unsigned_abs().saturating_mul(1000);
        Unit::try_from(magnitude.isqrt()).unwrap_or(ONE)
    }
}

/// The product of two thousandths, as thousandths.
const fn mul(left: Unit, right: Unit) -> Unit {
    left.saturating_mul(right).wrapping_div(ONE)
}

/// Reads a `transform` attribute: any number of functions, applied left to
/// right as SVG applies them.
pub(crate) fn parse(text: &str) -> Matrix {
    let mut matrix = Matrix::IDENTITY;
    let mut rest = text;
    while let Some((head, tail)) = rest.split_once('(') {
        let Some((arguments, next)) = tail.split_once(')') else {
            break;
        };
        let name = head
            .trim_matches(|character: char| !character.is_ascii_alphabetic())
            .to_ascii_lowercase();
        let values = number::list(arguments);
        if let Some(step) = function(&name, &values) {
            matrix = step.then(matrix);
        }
        rest = next;
    }
    matrix
}

/// One transform function, if it is one this crate draws.
fn function(name: &str, values: &[Unit]) -> Option<Matrix> {
    let at = |index: usize| values.get(index).copied();
    match name {
        "translate" => Some(Matrix::translation(at(0)?, at(1).unwrap_or(0))),
        "scale" => {
            let x = at(0)?;
            Some(Matrix::scaling(x, at(1).unwrap_or(x)))
        }
        "matrix" => Some(Matrix::new(at(0)?, at(1)?, at(2)?, at(3)?, at(4)?, at(5)?)),
        "rotate" => {
            let (sine, cosine) = turn(at(0)?);
            let rotation = Matrix::new(cosine, sine, sine.saturating_neg(), cosine, 0, 0);
            match (at(1), at(2)) {
                (Some(x), Some(y)) => Some(
                    Matrix::translation(x.saturating_neg(), y.saturating_neg())
                        .then(rotation)
                        .then(Matrix::translation(x, y)),
                ),
                _ => Some(rotation),
            }
        }
        _ => None,
    }
}

/// The sine and cosine of an angle in degrees, for the angles a drawing
/// is turned by. An angle that is not a multiple of ninety degrees is
/// read as the nearest one: a figure in these documents is turned by a
/// quarter, a half, or not at all, and a table of sines is a table this
/// crate would rather not carry.
const fn turn(degrees: Unit) -> (Unit, Unit) {
    let quarters = degrees
        .saturating_add(45_000)
        .div_euclid(90_000)
        .rem_euclid(4);
    match quarters {
        1 => (ONE, 0),
        2 => (0, ONE.saturating_neg()),
        3 => (ONE.saturating_neg(), 0),
        _ => (0, ONE),
    }
}
