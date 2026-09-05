// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The group of P-256 in Jacobian coordinates.
//!
//! A point is `(X, Y, Z)` with `x = X / Z^2` and `y = Y / Z^3`, and `Z = 0`
//! is the neutral element. Working projectively keeps the inversion out of
//! the addition and the doubling; one inversion at the end converts back.
//!
//! Everything here serves verification, where the point and the scalar are
//! public, so the multiplication is an ordinary double-and-add and the
//! special cases of the addition are branches.
//!
//! Invariant: every `Point` satisfies the curve equation, because the only
//! constructor from coordinates checks it and the operations preserve it.

use crate::p256::field::{Element, Prime};

/// An element of the field of the curve.
type Fp = Element<Prime>;

/// The curve constant `b`, in Montgomery form.
const B: Fp = Fp::from_montgomery([
    0xd89c_df62_29c4_bddf,
    0xacf0_05cd_7884_3090,
    0xe5a2_20ab_f721_2ed6,
    0xdc30_061d_0487_4834,
]);

/// The `x` coordinate of the base point, in Montgomery form.
const GENERATOR_X: Fp = Fp::from_montgomery([
    0x79e7_30d4_18a9_143c,
    0x75ba_95fc_5fed_b601,
    0x79fb_732b_7762_2510,
    0x1890_5f76_a537_55c6,
]);

/// The `y` coordinate of the base point, in Montgomery form.
const GENERATOR_Y: Fp = Fp::from_montgomery([
    0xddf2_5357_ce95_560a,
    0x8b4a_b8e4_ba19_e45c,
    0xd2e8_8688_dd21_f325,
    0x8571_ff18_2588_5d85,
]);

/// A point of the curve in Jacobian coordinates.
///
/// There is deliberately no equality: the same point has many
/// representations, one per choice of `Z`, so a comparison of coordinates
/// would answer a different question than the caller asked. Compare
/// [`Point::to_affine`] instead.
#[derive(Clone, Copy, Debug)]
pub struct Point {
    /// `x * Z^2`.
    x: Fp,
    /// `y * Z^3`.
    y: Fp,
    /// The denominator; zero for the neutral element.
    z: Fp,
}

impl Point {
    /// The neutral element.
    #[must_use]
    pub const fn identity() -> Point {
        Point {
            x: Fp::one(),
            y: Fp::one(),
            z: Fp::zero(),
        }
    }

    /// The base point.
    #[must_use]
    pub const fn generator() -> Point {
        Point {
            x: GENERATOR_X,
            y: GENERATOR_Y,
            z: Fp::one(),
        }
    }

    /// The point with the given affine coordinates, or `None` when they do
    /// not satisfy `y^2 = x^3 - 3x + b`.
    #[must_use]
    pub fn from_affine(x: Fp, y: Fp) -> Option<Point> {
        let three = Fp::one().double().add(Fp::one());
        let right = x.square().mul(x).sub(three.mul(x)).add(B);
        if y.square() == right {
            Some(Point { x, y, z: Fp::one() })
        } else {
            None
        }
    }

    /// Whether the point is the neutral element.
    #[must_use]
    pub fn is_identity(self) -> bool {
        self.z.is_zero()
    }

    /// Twice the point, by the formula for a curve whose `a` is minus
    /// three.
    #[must_use]
    pub fn double(self) -> Point {
        let delta = self.z.square();
        let gamma = self.y.square();
        let beta = self.x.mul(gamma);
        let difference = self.x.sub(delta);
        let sum = self.x.add(delta);
        let alpha = difference.mul(sum);
        let alpha = alpha.double().add(alpha);

        let eight_beta = beta.double().double().double();
        let x = alpha.square().sub(eight_beta);
        let z = self.y.add(self.z).square().sub(gamma).sub(delta);
        let four_beta = beta.double().double();
        let y = alpha
            .mul(four_beta.sub(x))
            .sub(gamma.square().double().double().double());
        Point { x, y, z }
    }

    /// The sum.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; field operations are named after what they are"
    )]
    #[expect(
        clippy::many_single_char_names,
        reason = "the intermediate values are named as in the formula this follows"
    )]
    pub fn add(self, other: Point) -> Point {
        if self.is_identity() {
            return other;
        }
        if other.is_identity() {
            return self;
        }

        let z1z1 = self.z.square();
        let z2z2 = other.z.square();
        let u1 = self.x.mul(z2z2);
        let u2 = other.x.mul(z1z1);
        let s1 = self.y.mul(other.z).mul(z2z2);
        let s2 = other.y.mul(self.z).mul(z1z1);
        let h = u2.sub(u1);
        let r = s2.sub(s1).double();

        if h.is_zero() {
            if r.is_zero() {
                return self.double();
            }
            return Point::identity();
        }

        let i = h.double().square();
        let j = h.mul(i);
        let v = u1.mul(i);
        let x = r.square().sub(j).sub(v.double());
        let y = r.mul(v.sub(x)).sub(s1.mul(j).double());
        let z = self.z.add(other.z).square().sub(z1z1).sub(z2z2).mul(h);
        Point { x, y, z }
    }

    /// The multiple by the big-endian `scalar`, by doubling and adding.
    /// The scalar is public wherever this crate multiplies.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; field operations are named after what they are"
    )]
    pub fn mul(self, scalar: &[u8; 32]) -> Point {
        let mut result = Point::identity();
        for position in (0..256u32).rev() {
            result = result.double();
            if bit_of(scalar, position) == 1 {
                result = result.add(self);
            }
        }
        result
    }

    /// The affine coordinates, or `None` for the neutral element.
    #[must_use]
    pub fn to_affine(self) -> Option<(Fp, Fp)> {
        if self.is_identity() {
            return None;
        }
        let inverse = self.z.invert();
        let square = inverse.square();
        let cube = square.mul(inverse);
        Some((self.x.mul(square), self.y.mul(cube)))
    }
}

/// Bit `position` of a big-endian value, counted from the least
/// significant.
fn bit_of(bytes: &[u8; 32], position: u32) -> u8 {
    let from_top = 31u32.wrapping_sub(position.wrapping_shr(3));
    let within = position & 7;
    let mut value = 0u8;
    for (index, candidate) in (0u32..).zip(bytes) {
        if index == from_top {
            value = *candidate;
        }
    }
    value.wrapping_shr(within) & 1
}
