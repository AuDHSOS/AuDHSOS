// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! The elementary functions of binary64: the square root, the
//! exponential, the logarithms, the three circular functions with their
//! inverses, and the three hyperbolic functions with theirs.
//!
//! Each one reduces its argument to a range a fixed-length series covers
//! and sums that series in two-component arithmetic, so the work is
//! bounded and no host math library is called.

use crate::{LN2, Wide, exponential, logarithm, power_of_two};

/// `ln(10)` to about thirty digits, which the logarithm of base ten
/// divides by.
const LN10: Wide = Wide(core::f64::consts::LN_10, -2.170_756_223_382_249_4e-16);

/// A quarter turn to about thirty digits, which the reduction of a
/// circular function subtracts a multiple of.
const HALF_PI: Wide = Wide(core::f64::consts::FRAC_PI_2, 6.123_233_995_736_766e-17);

/// Half a turn to about thirty digits, which the quadrants of `atan2`
/// add.
const PI: Wide = Wide(core::f64::consts::PI, 1.224_646_799_147_353_2e-16);

/// One as a two-component number, which the series here add and divide
/// by.
const ONE: Wide = Wide(1.0, 0.0);

/// The square root.
///
/// The iteration is Newton's, from a seed of one digit, and the last step
/// is taken in two-component arithmetic, so the answer carries the whole
/// significand. Every argument takes the same six steps.
#[must_use]
pub fn sqrt(x: f64) -> f64 {
    if x.is_nan() || x == f64::INFINITY || x == 0.0 {
        // The sign of a zero is kept, which `sqrt(-0.0)` is `-0.0` for.
        return x;
    }
    if x < 0.0 {
        return f64::NAN;
    }
    // The argument is scaled to `[1, 4)` by an even power of two, so the
    // root is scaled back by half that power.
    let (significand, twos) = split_even(x);
    let mut root = 0.833 + significand * (0.283 - significand * 0.026);
    for _ in 0..5 {
        root = root.midpoint(significand / root);
    }
    let held = narrow(root_step(Wide(significand, 0.0), root));
    scaled(held, twos / 2)
}

/// One double as a significand in `[1, 4)` and an even power of two,
/// which is the scaling a square root undoes by halving the power.
fn split_even(x: f64) -> (f64, i32) {
    let mut significand = x;
    let mut twos = 0_i32;
    // A subnormal is scaled up first, because its exponent field holds
    // nought and says nothing about its size.
    if significand < f64::MIN_POSITIVE {
        significand *= 18_014_398_509_481_984.0;
        twos = -54;
    }
    let bits = significand.to_bits();
    let exponent = i32::try_from((bits >> 52) & 0x7ff)
        .unwrap_or(0)
        .saturating_sub(1023);
    let mantissa = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    let odd = exponent.rem_euclid(2);
    (
        mantissa * power_of_two(odd),
        twos.saturating_add(exponent.saturating_sub(odd)),
    )
}

/// One Newton step over a root that is right to fourteen digits, which
/// carries it to thirty.
fn root_step(x: Wide, root: f64) -> Wide {
    let wide = Wide(root, 0.0);
    let residual = x.add(wide.mul(wide).neg());
    wide.add(residual.div(Wide(root + root, 0.0)))
}

/// The square root of a two-component argument.
fn wide_sqrt(x: Wide) -> Wide {
    root_step(x, sqrt(narrow(x)))
}

/// One two-component number as the double nearest it.
///
/// A head of nought is answered as it stands, because adding the tail to
/// it would lose the sign a zero carries.
fn narrow(x: Wide) -> f64 {
    if x.0 == 0.0 {
        return x.0;
    }
    x.0 + x.1
}

/// One double times a power of two, which no multiplication rounds
/// because the power is exact until the answer leaves the normal range.
fn scaled(value: f64, twos: i32) -> f64 {
    if twos > 1023 {
        return (value * 2.0) * power_of_two(twos.saturating_sub(1));
    }
    if twos < -1022 {
        return (value * power_of_two(twos.saturating_add(1022))) * f64::MIN_POSITIVE;
    }
    value * power_of_two(twos)
}

/// `e` raised to `x`.
#[must_use]
pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 710.0 {
        return f64::INFINITY;
    }
    if x < -746.0 {
        return 0.0;
    }
    exponential(Wide(x, 0.0))
}

/// The natural logarithm.
#[must_use]
pub fn ln(x: f64) -> f64 {
    based(x, ONE)
}

/// The logarithm of base two, which is the exponent itself for a power of
/// two.
#[must_use]
pub fn log2(x: f64) -> f64 {
    based(x, LN2)
}

/// The logarithm of base ten.
#[must_use]
pub fn log10(x: f64) -> f64 {
    based(x, LN10)
}

/// The natural logarithm divided by `base`, which is the logarithm of
/// the base that number is the natural logarithm of.
fn based(x: f64, base: Wide) -> f64 {
    if x.is_nan() || x == f64::INFINITY {
        return x;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x < 0.0 {
        return f64::NAN;
    }
    narrow(logarithm(x).div(base))
}

/// The natural logarithm of a two-component argument.
///
/// The tail is at most one part in 2^53 of the head, so the logarithm of
/// their quotient is that fraction itself: every term after the first
/// falls below what two components hold.
fn wide_logarithm(x: Wide) -> Wide {
    logarithm(x.0).add(Wide(x.1 / x.0, 0.0))
}

/// The sine.
#[must_use]
pub fn sin(x: f64) -> f64 {
    if x == 0.0 {
        // The sign of a zero is kept, which `sin(-0.0)` is `-0.0` for.
        return x;
    }
    circular(x, 0)
}

/// The cosine.
#[must_use]
pub fn cos(x: f64) -> f64 {
    circular(x, 1)
}

/// The sine where `turn` is nought and the cosine where it is one, which
/// are the same two series read a quarter turn apart.
fn circular(x: f64, turn: u32) -> f64 {
    if !x.is_finite() {
        return if x.is_nan() { x } else { f64::NAN };
    }
    let (quadrant, rest) = quadrant_of(x);
    narrow(match quadrant.wrapping_add(turn) % 4 {
        0 => sine_series(rest),
        1 => cosine_series(rest),
        2 => sine_series(rest).neg(),
        _ => cosine_series(rest).neg(),
    })
}

/// The tangent, which is the sine over the cosine.
#[must_use]
pub fn tan(x: f64) -> f64 {
    if x == 0.0 {
        return x;
    }
    if !x.is_finite() {
        return if x.is_nan() { x } else { f64::NAN };
    }
    let (quadrant, rest) = quadrant_of(x);
    let sine = sine_series(rest);
    let cosine = cosine_series(rest);
    // A quarter turn on turns the quotient over and changes its sign.
    let (top, bottom) = if quadrant % 2 == 0 {
        (sine, cosine)
    } else {
        (cosine.neg(), sine)
    };
    narrow(top.div(bottom))
}

/// One argument as a count of quarter turns and what is left of it, which
/// is an eighth of a turn at most.
///
/// The count is answered modulo four, so a circular function reads what
/// the argument holds of one turn.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::as_conversions,
    reason = "the count is read modulo four and a count past the integers leaves a bounded rest"
)]
fn quadrant_of(x: f64) -> (u32, Wide) {
    let turns = x * core::f64::consts::FRAC_2_PI;
    let count = (turns + if turns >= 0.0 { 0.5 } else { -0.5 }) as i64;
    let rest = Wide(x, 0.0).add(HALF_PI.mul(Wide(count as f64, 0.0)).neg());
    (u32::try_from(count.rem_euclid(4)).unwrap_or(0), rest)
}

/// The sine of an eighth of a turn at most, which is its Taylor series.
fn sine_series(x: Wide) -> Wide {
    let square = x.mul(x);
    let mut term = x;
    let mut sum = x;
    for k in 1i32..14 {
        term = term.mul(square).div(Wide(-factorial_step(k, 1), 0.0));
        sum = sum.add(term);
    }
    sum
}

/// The cosine of an eighth of a turn at most, which is its Taylor series.
fn cosine_series(x: Wide) -> Wide {
    let square = x.mul(x);
    let mut term = ONE;
    let mut sum = term;
    for k in 1i32..14 {
        term = term.mul(square).div(Wide(-factorial_step(k, 0), 0.0));
        sum = sum.add(term);
    }
    sum
}

/// What one step of a Taylor series of the sine or the cosine divides by:
/// the two factors the factorial gains, which are `2k + odd` and the
/// number under it.
fn factorial_step(k: i32, odd: i32) -> f64 {
    let below = k.saturating_mul(2).saturating_add(odd);
    f64::from(below.saturating_mul(below.saturating_sub(1)))
}

/// The arc tangent.
#[must_use]
pub fn atan(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        return x;
    }
    if x.is_infinite() {
        return if x > 0.0 { HALF_PI.0 } else { -HALF_PI.0 };
    }
    narrow(arc_tangent(Wide(x, 0.0)))
}

/// The arc tangent of a two-component argument.
///
/// `atan(x)` is `2 atan(x / (1 + sqrt(1 + x²)))`, so three halvings hold
/// the argument under a thirty-second of a turn, where sixteen terms of
/// the series carry thirty digits. A magnitude over one is read as the
/// angle its reciprocal leaves of a quarter turn, because the series
/// wants a fraction.
fn arc_tangent(x: Wide) -> Wide {
    let large = x.0.abs() > 1.0;
    let mut held = if large { ONE.div(x) } else { x };
    for _ in 0..3 {
        held = held.div(ONE.add(wide_sqrt(ONE.add(held.mul(held)))));
    }
    let sum = arc_tangent_series(held).mul(Wide(8.0, 0.0));
    if large {
        let quarter = if x.0 < 0.0 { HALF_PI.neg() } else { HALF_PI };
        return quarter.add(sum.neg());
    }
    sum
}

/// The arc tangent of a thirty-second of a turn at most, which is its
/// Gregory series.
fn arc_tangent_series(x: Wide) -> Wide {
    let square = x.mul(x);
    let mut term = x;
    let mut sum = x;
    for k in 1i32..17 {
        term = term.mul(square).neg();
        let below = f64::from(k.saturating_mul(2).saturating_add(1));
        sum = sum.add(term.div(Wide(below, 0.0)));
    }
    sum
}

/// The arc sine.
#[expect(
    clippy::float_cmp,
    reason = "the ends of the domain are the exact values the answer is named for"
)]
#[must_use]
pub fn asin(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x.abs() > 1.0 {
        return f64::NAN;
    }
    if x.abs() == 1.0 {
        return if x > 0.0 { HALF_PI.0 } else { -HALF_PI.0 };
    }
    narrow(arc_sine(Wide(x, 0.0)))
}

/// The arc sine of a two-component argument under one in magnitude, which
/// is the arc tangent of the argument over the other side of the triangle
/// it stands in.
fn arc_sine(x: Wide) -> Wide {
    arc_tangent(x.div(wide_sqrt(ONE.add(x.mul(x).neg()))))
}

/// The arc cosine.
///
/// An argument over a half is read as twice the arc sine of the half
/// angle, where the distance from one is still held in full, and every
/// other one as the quarter turn the arc sine leaves.
#[expect(
    clippy::float_cmp,
    reason = "the ends of the domain are the exact values the answer is named for"
)]
#[must_use]
pub fn acos(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x.abs() > 1.0 {
        return f64::NAN;
    }
    if x == 1.0 {
        return 0.0;
    }
    if x == -1.0 {
        return PI.0;
    }
    let held = Wide(x, 0.0);
    narrow(if x > 0.5 {
        arc_sine(wide_sqrt(ONE.add(held.neg()).div(Wide(2.0, 0.0)))).mul(Wide(2.0, 0.0))
    } else if x < -0.5 {
        PI.add(arc_sine(wide_sqrt(ONE.add(held).div(Wide(2.0, 0.0)))).mul(Wide(-2.0, 0.0)))
    } else {
        HALF_PI.add(arc_sine(held).neg())
    })
}

/// The angle of the point `(x, y)`, which is the arc tangent of the
/// quotient of the two in the quadrant their signs name.
#[must_use]
pub fn atan2(y: f64, x: f64) -> f64 {
    if y.is_nan() || x.is_nan() {
        return f64::NAN;
    }
    if y == 0.0 {
        // A zero is signed, and the side the other argument lies on says
        // whether the angle is nought or half a turn.
        return match (x.is_sign_negative(), y.is_sign_negative()) {
            (false, _) => y,
            (true, false) => PI.0,
            (true, true) => -PI.0,
        };
    }
    if x == 0.0 {
        return if y > 0.0 { HALF_PI.0 } else { -HALF_PI.0 };
    }
    if !y.is_finite() || !x.is_finite() {
        return unbounded_angle(y, x);
    }
    let ratio = Wide(y, 0.0).div(Wide(x, 0.0));
    // A quotient the range does not hold leaves a quarter turn, because
    // what the angle stands from it is under one part in 2^1000.
    if !ratio.0.is_finite() {
        return if y > 0.0 { HALF_PI.0 } else { -HALF_PI.0 };
    }
    // A quotient the range holds as a zero leaves the angle at nought or
    // at half a turn, and a zero carries the sign of the height.
    if ratio.0 == 0.0 {
        if x > 0.0 {
            return y * 0.0;
        }
        return if y > 0.0 { PI.0 } else { -PI.0 };
    }
    let held = arc_tangent(ratio);
    narrow(if x > 0.0 {
        held
    } else if y > 0.0 {
        PI.add(held)
    } else {
        PI.neg().add(held)
    })
}

/// The angle of a point one of whose sides is boundless, which is an
/// eighth of a turn where both are, a quarter turn where the height is,
/// and nought or half a turn where the width is.
fn unbounded_angle(y: f64, x: f64) -> f64 {
    let sign = if y > 0.0 { 1.0 } else { -1.0 };
    if y.is_infinite() && x.is_infinite() {
        return sign
            * if x > 0.0 {
                HALF_PI.0 / 2.0
            } else {
                HALF_PI.0 * 1.5
            };
    }
    if y.is_infinite() {
        return sign * HALF_PI.0;
    }
    sign * if x > 0.0 { 0.0 } else { PI.0 }
}

/// The hyperbolic sine.
#[must_use]
pub fn sinh(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    // The difference of two exponentials loses digits near nought, where
    // the series holds them.
    if x.abs() < 0.5 {
        return narrow(hyperbolic_series(Wide(x, 0.0), 1));
    }
    narrow(halved(x, true))
}

/// The hyperbolic cosine.
#[must_use]
pub fn cosh(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x.is_infinite() {
        return f64::INFINITY;
    }
    narrow(halved(x, false))
}

/// The hyperbolic tangent, which is one in magnitude from a large
/// argument onward.
#[must_use]
pub fn tanh(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        return x;
    }
    if x.abs() > 20.0 {
        return if x > 0.0 { 1.0 } else { -1.0 };
    }
    if x.abs() < 0.5 {
        let held = Wide(x, 0.0);
        return narrow(hyperbolic_series(held, 1).div(hyperbolic_series(held, 0)));
    }
    narrow(halved(x, true).div(halved(x, false)))
}

/// Half the sum of the exponential and its reciprocal, and half their
/// difference where `odd`.
fn halved(x: f64, odd: bool) -> Wide {
    let up = Wide(exp(x), 0.0);
    let down = Wide(exp(-x), 0.0);
    let sum = if odd {
        up.add(down.neg())
    } else {
        up.add(down)
    };
    sum.div(Wide(2.0, 0.0))
}

/// The hyperbolic sine of a small argument where `odd` is one, and its
/// cosine where `odd` is nought, each as its Taylor series.
fn hyperbolic_series(x: Wide, odd: i32) -> Wide {
    let square = x.mul(x);
    let mut term = if odd == 1 { x } else { ONE };
    let mut sum = term;
    for k in 1i32..14 {
        term = term.mul(square).div(Wide(factorial_step(k, odd), 0.0));
        sum = sum.add(term);
    }
    sum
}

/// The inverse hyperbolic sine, which is the logarithm of the argument
/// and the other side of the triangle it stands in.
#[must_use]
pub fn asinh(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let held = Wide(x.abs(), 0.0);
    let inside = held.add(wide_sqrt(ONE.add(held.mul(held))));
    let answer = narrow(wide_logarithm(inside));
    if x < 0.0 { -answer } else { answer }
}

/// The inverse hyperbolic cosine, which no argument under one has.
#[expect(
    clippy::float_cmp,
    reason = "the ends of the domain are the exact values the answer is named for"
)]
#[must_use]
pub fn acosh(x: f64) -> f64 {
    if x.is_nan() || x == f64::INFINITY {
        return x;
    }
    if x < 1.0 {
        return f64::NAN;
    }
    if x == 1.0 {
        return 0.0;
    }
    let held = Wide(x, 0.0);
    let inside = held.add(wide_sqrt(held.mul(held).add(ONE.neg())));
    narrow(wide_logarithm(inside))
}

/// The inverse hyperbolic tangent, which no argument over one in
/// magnitude has and which is boundless at one.
#[expect(
    clippy::float_cmp,
    reason = "the ends of the domain are the exact values the answer is named for"
)]
#[must_use]
pub fn atanh(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        return x;
    }
    if x.abs() > 1.0 {
        return f64::NAN;
    }
    if x.abs() == 1.0 {
        return if x > 0.0 {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        };
    }
    let held = Wide(x, 0.0);
    narrow(wide_logarithm(ONE.add(held).div(ONE.add(held.neg()))).div(Wide(2.0, 0.0)))
}
