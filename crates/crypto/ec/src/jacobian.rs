// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The group of a short Weierstrass curve whose `a` is minus three, in
//! Jacobian coordinates.
//!
//! A point is `(X, Y, Z)` with `x = X / Z^2` and `y = Y / Z^3`, and `Z = 0`
//! is the neutral element. Working projectively keeps the inversion out of
//! the addition and the doubling; one inversion at the end converts back.
//!
//! Both curves this crate verifies over — P-256 and P-384 — have `a = -3`,
//! which is what the doubling formula here assumes. A curve with another
//! `a` cannot use this module, and [`Curve`] carries no `a` to suggest
//! otherwise.
//!
//! Everything here serves verification, where the point and the scalar are
//! public, so the multiplication is an ordinary double-and-add and the
//! special cases of the addition are branches.
//!
//! Invariant: every `Point` satisfies the curve equation, because the only
//! constructor from coordinates checks it and the operations preserve it.

use crate::montgomery::{Element, Params};

/// A curve `y^2 = x^3 - 3x + b` over the field `Field`.
pub trait Curve<const N: usize>: Copy {
    /// The field the curve lives in.
    type Field: Params<N>;
    /// The constant `b`, in Montgomery form.
    const B: [u64; N];
    /// The `x` coordinate of the base point, in Montgomery form.
    const GENERATOR_X: [u64; N];
    /// The `y` coordinate of the base point, in Montgomery form.
    const GENERATOR_Y: [u64; N];
}

/// An element of the field of `C`.
type Fp<C, const N: usize, const BYTES: usize> = Element<<C as Curve<N>>::Field, N, BYTES>;

/// A point of the curve `C` in Jacobian coordinates.
///
/// There is deliberately no equality: the same point has many
/// representations, one per choice of `Z`, so a comparison of coordinates
/// would answer a different question than the caller asked. Compare
/// [`Point::to_affine`] instead.
#[derive(Clone, Copy, Debug)]
pub struct Point<C: Curve<N>, const N: usize, const BYTES: usize> {
    /// `x * Z^2`.
    x: Fp<C, N, BYTES>,
    /// `y * Z^3`.
    y: Fp<C, N, BYTES>,
    /// The denominator; zero for the neutral element.
    z: Fp<C, N, BYTES>,
}

impl<C: Curve<N>, const N: usize, const BYTES: usize> Point<C, N, BYTES> {
    /// The neutral element.
    #[must_use]
    pub const fn identity() -> Point<C, N, BYTES> {
        Point {
            x: Fp::<C, N, BYTES>::one(),
            y: Fp::<C, N, BYTES>::one(),
            z: Fp::<C, N, BYTES>::zero(),
        }
    }

    /// The base point.
    #[must_use]
    pub const fn generator() -> Point<C, N, BYTES> {
        Point {
            x: Fp::<C, N, BYTES>::from_montgomery(C::GENERATOR_X),
            y: Fp::<C, N, BYTES>::from_montgomery(C::GENERATOR_Y),
            z: Fp::<C, N, BYTES>::one(),
        }
    }

    /// The point with the given affine coordinates, or `None` when they do
    /// not satisfy `y^2 = x^3 - 3x + b`.
    #[must_use]
    pub fn from_affine(x: Fp<C, N, BYTES>, y: Fp<C, N, BYTES>) -> Option<Point<C, N, BYTES>> {
        let b = Fp::<C, N, BYTES>::from_montgomery(C::B);
        let three = Fp::<C, N, BYTES>::one()
            .double()
            .add(Fp::<C, N, BYTES>::one());
        let right = x.square().mul(x).sub(three.mul(x)).add(b);
        if y.square() == right {
            Some(Point {
                x,
                y,
                z: Fp::<C, N, BYTES>::one(),
            })
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
    pub fn double(self) -> Point<C, N, BYTES> {
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
    pub fn add(self, other: Point<C, N, BYTES>) -> Point<C, N, BYTES> {
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
    pub fn mul(self, scalar: &[u8; BYTES]) -> Point<C, N, BYTES> {
        let bits = u32::try_from(BYTES).unwrap_or(0).wrapping_mul(8);
        let mut result = Point::identity();
        for position in (0..bits).rev() {
            result = result.double();
            if bit_of(scalar, position) == 1 {
                result = result.add(self);
            }
        }
        result
    }

    /// The affine coordinates, or `None` for the neutral element.
    #[must_use]
    pub fn to_affine(self) -> Option<(Fp<C, N, BYTES>, Fp<C, N, BYTES>)> {
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
fn bit_of<const BYTES: usize>(bytes: &[u8; BYTES], position: u32) -> u8 {
    let last = u32::try_from(BYTES).unwrap_or(0).wrapping_sub(1);
    let from_top = last.wrapping_sub(position.wrapping_shr(3));
    let within = position & 7;
    let mut value = 0u8;
    for (index, candidate) in (0u32..).zip(bytes) {
        if index == from_top {
            value = *candidate;
        }
    }
    value.wrapping_shr(within) & 1
}
