// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Ed25519 verification from RFC 8032.
//!
//! Everything a verifier touches is public: the key, the message, and the
//! signature are all on the wire. The point arithmetic here therefore
//! branches freely, which makes it shorter and easier to compare against
//! the formulas it comes from. The constant-time work of this crate is in
//! [`mod@crate::x25519`], where the scalar is secret.
//!
//! Verification is the strict form: the encodings of `R`, of the public
//! key, and of `S` must be canonical, the public key must not be of small
//! order, and the equation is checked without the cofactor. A signature
//! that only passes the cofactored equation is rejected, which is what
//! makes a signature one string of bytes rather than a family of them.
//!
//! Invariant: `Point` always holds a point of the curve in extended
//! coordinates, `x = X/Z`, `y = Y/Z`, `xy = T/Z`. The only constructor
//! from bytes checks the curve equation.

use crypto_hash::Sha512;

use crate::error::EcError;
use crate::fe25519::Fe;
use crate::scalar::Scalar;

/// Bytes of a public key.
pub const PUBLIC_LEN: usize = 32;
/// Bytes of a signature.
pub const SIGNATURE_LEN: usize = 64;

/// The curve constant `d`, which is `-121665 / 121666`.
const D: Fe = Fe::from_limbs([
    0x0003_4dca_1359_78a3,
    0x0001_a828_3b15_6ebd,
    0x0005_e7a2_6001_c029,
    0x0007_39c6_63a0_3cbb,
    0x0005_2036_cee2_b6ff,
]);

/// Twice the curve constant, as the addition formula uses it.
const D2: Fe = Fe::from_limbs([
    0x0006_9b94_26b2_f159,
    0x0003_5050_762a_dd7a,
    0x0003_cf44_c003_8052,
    0x0006_738c_c740_7977,
    0x0002_406d_9dc5_6dff,
]);

/// A square root of minus one, for the decompression.
const SQRT_M1: Fe = Fe::from_limbs([
    0x0006_1b27_4a0e_a0b0,
    0x0000_d5a5_fc8f_189d,
    0x0007_ef5e_9cbd_0c60,
    0x0007_8595_a680_4c9e,
    0x0002_b832_4804_fc1d,
]);

/// The `x` coordinate of the base point.
const BASE_X: Fe = Fe::from_limbs([
    0x0006_2d60_8f25_d51a,
    0x0004_12a4_b4f6_592a,
    0x0007_5b71_71a4_b31d,
    0x0001_ff60_5271_18fe,
    0x0002_1693_6d3c_d6e5,
]);

/// The `y` coordinate of the base point, which is `4 / 5`.
const BASE_Y: Fe = Fe::from_limbs([
    0x0006_6666_6666_6658,
    0x0004_cccc_cccc_cccc,
    0x0001_9999_9999_9999,
    0x0003_3333_3333_3333,
    0x0006_6666_6666_6666,
]);

/// A point of the curve in extended coordinates.
#[derive(Clone, Copy)]
pub struct Point {
    /// `x * Z`.
    x: Fe,
    /// `y * Z`.
    y: Fe,
    /// The common denominator.
    z: Fe,
    /// `x * y * Z`.
    t: Fe,
}

impl Point {
    /// The neutral element.
    pub const IDENTITY: Point = Point {
        x: Fe::ZERO,
        y: Fe::ONE,
        z: Fe::ONE,
        t: Fe::ZERO,
    };

    /// The base point.
    #[must_use]
    pub fn base() -> Point {
        Point {
            x: BASE_X,
            y: BASE_Y,
            z: Fe::ONE,
            t: BASE_X.mul(BASE_Y),
        }
    }

    /// The point the bytes encode, or `None` when they are not the
    /// canonical encoding of a point.
    #[must_use]
    pub fn decompress(bytes: &[u8; PUBLIC_LEN]) -> Option<Point> {
        let sign = bytes[31] >> 7;
        let mut encoded = *bytes;
        encoded[31] &= 0x7F;

        let y = Fe::from_bytes(&encoded);
        if y.to_bytes() != encoded {
            // The `y` coordinate was at or above the prime.
            return None;
        }

        // Solve `x^2 = (y^2 - 1) / (d y^2 + 1)`.
        let yy = y.square();
        let u = yy.sub(Fe::ONE);
        let v = yy.mul(D).add(Fe::ONE);
        let v3 = v.square().mul(v);
        let v7 = v3.square().mul(v);
        let mut x = u.mul(v3).mul(u.mul(v7).pow22523());

        let check = v.mul(x.square());
        if check.sub(u).is_zero().is_true() {
            // The candidate is the root.
        } else if check.add(u).is_zero().is_true() {
            x = x.mul(SQRT_M1);
        } else {
            return None;
        }

        if x.is_zero().is_true() && sign == 1 {
            // The only encoding of a zero `x` has the sign bit clear.
            return None;
        }
        if u8::from(x.is_negative().is_true()) != sign {
            x = x.negate();
        }

        Some(Point {
            x,
            y,
            z: Fe::ONE,
            t: x.mul(y),
        })
    }

    /// The canonical encoding: `y` with the sign of `x` in the highest bit.
    #[must_use]
    pub fn compress(self) -> [u8; PUBLIC_LEN] {
        let inverse = self.z.invert();
        let x = self.x.mul(inverse);
        let y = self.y.mul(inverse);
        let mut bytes = y.to_bytes();
        bytes[31] |= x.is_negative().mask_u8() & 0x80;
        bytes
    }

    /// The sum, by the addition formula for extended coordinates.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; group and scalar operations are named after what \
                  they are"
    )]
    #[expect(
        clippy::many_single_char_names,
        reason = "the intermediate values are named as in the formula this follows"
    )]
    pub fn add(self, other: Point) -> Point {
        let a = self.y.sub(self.x).mul(other.y.sub(other.x));
        let b = self.y.add(self.x).mul(other.y.add(other.x));
        let c = self.t.mul(D2).mul(other.t);
        let d = self.z.mul(other.z);
        let d = d.add(d);
        let e = b.sub(a);
        let f = d.sub(c);
        let g = d.add(c);
        let h = b.add(a);
        Point {
            x: e.mul(f),
            y: g.mul(h),
            z: f.mul(g),
            t: e.mul(h),
        }
    }

    /// Twice the point.
    #[must_use]
    #[expect(
        clippy::many_single_char_names,
        reason = "the intermediate values are named as in the formula this follows"
    )]
    pub fn double(self) -> Point {
        let a = self.x.square();
        let b = self.y.square();
        let c = self.z.square();
        let c = c.add(c);
        let d = a.negate();
        let e = self.x.add(self.y).square().sub(a).sub(b);
        let g = d.add(b);
        let f = g.sub(c);
        let h = d.sub(b);
        Point {
            x: e.mul(f),
            y: g.mul(h),
            z: f.mul(g),
            t: e.mul(h),
        }
    }

    /// The multiple by `scalar`, by doubling and adding. The scalar is
    /// public wherever this crate multiplies, so the branch is allowed.
    #[must_use]
    #[expect(
        clippy::should_implement_trait,
        reason = "the operator traits would invite `+` and `*` at every call site, where the workspace\
                  denies unchecked arithmetic; group and scalar operations are named after what \
                  they are"
    )]
    pub fn mul(self, scalar: Scalar) -> Point {
        let mut result = Point::IDENTITY;
        for position in (0..256u32).rev() {
            result = result.double();
            if scalar.bit(position) == 1 {
                result = result.add(self);
            }
        }
        result
    }

    /// Whether the point has order eight or less, which is what a key that
    /// carries no signal looks like.
    #[must_use]
    pub fn is_small_order(self) -> bool {
        self.double().double().double().is_identity()
    }

    /// Whether the point is the neutral element.
    #[must_use]
    pub fn is_identity(self) -> bool {
        self.compress() == Point::IDENTITY.compress()
    }
}

/// Whether `signature` is a signature of `message` under `public`.
///
/// # Errors
///
/// [`EcError::InvalidPoint`] when the public key or the point `R` is not a
/// canonical encoding of a point, or the key is of small order;
/// [`EcError::InvalidScalar`] when `S` is not below the group order;
/// [`EcError::BadSignature`] when the equation does not hold.
pub fn verify(
    public: &[u8; PUBLIC_LEN],
    message: &[u8],
    signature: &[u8; SIGNATURE_LEN],
) -> Result<(), EcError> {
    let (left, right) = signature.split_at(32);
    let mut commitment = [0u8; 32];
    let mut response = [0u8; 32];
    for (slot, byte) in commitment.iter_mut().zip(left) {
        *slot = *byte;
    }
    for (slot, byte) in response.iter_mut().zip(right) {
        *slot = *byte;
    }

    let s = Scalar::from_canonical(&response).ok_or(EcError::InvalidScalar)?;
    let a = Point::decompress(public).ok_or(EcError::InvalidPoint)?;
    if a.is_small_order() {
        return Err(EcError::InvalidPoint);
    }
    let r = Point::decompress(&commitment).ok_or(EcError::InvalidPoint)?;

    let mut hash = Sha512::new();
    hash.update(&commitment);
    hash.update(public);
    hash.update(message);
    let k = Scalar::from_wide(&hash.finish());

    let expected = Point::base().mul(s);
    let found = r.add(a.mul(k));
    if expected.compress() == found.compress() {
        Ok(())
    } else {
        Err(EcError::BadSignature)
    }
}

/// The public key of `secret`.
#[cfg(any(test, feature = "test-signing"))]
#[must_use]
pub fn public_key(secret: &[u8; 32]) -> [u8; PUBLIC_LEN] {
    let (scalar, _) = expand(secret);
    Point::base().mul(scalar).compress()
}

/// The signature of `message` under `secret`.
///
/// Signing exists for the test data of this repository and is compiled
/// into product builds by no feature. It is deterministic, as RFC 8032
/// prescribes, so it needs no randomness.
#[cfg(any(test, feature = "test-signing"))]
#[must_use]
pub fn sign(secret: &[u8; 32], message: &[u8]) -> [u8; SIGNATURE_LEN] {
    let (scalar, prefix) = expand(secret);
    let public = Point::base().mul(scalar).compress();

    let mut hash = Sha512::new();
    hash.update(&prefix);
    hash.update(message);
    let r = Scalar::from_wide(&hash.finish());
    let commitment = Point::base().mul(r).compress();

    let mut hash = Sha512::new();
    hash.update(&commitment);
    hash.update(&public);
    hash.update(message);
    let k = Scalar::from_wide(&hash.finish());
    let s = r.add(k.mul(scalar));

    let mut signature = [0u8; SIGNATURE_LEN];
    for (slot, byte) in signature.iter_mut().zip(commitment) {
        *slot = byte;
    }
    for (slot, byte) in signature.iter_mut().skip(32).zip(s.to_bytes()) {
        *slot = byte;
    }
    signature
}

/// The scalar and the prefix a secret expands into.
#[cfg(any(test, feature = "test-signing"))]
fn expand(secret: &[u8; 32]) -> (Scalar, [u8; 32]) {
    let digest = Sha512::digest(secret);
    let (low, high) = digest.split_at(32);
    let mut clamped = [0u8; 32];
    for (slot, byte) in clamped.iter_mut().zip(low) {
        *slot = *byte;
    }
    clamped[0] &= 0xF8;
    clamped[31] &= 0x7F;
    clamped[31] |= 0x40;

    let mut prefix = [0u8; 32];
    for (slot, byte) in prefix.iter_mut().zip(high) {
        *slot = *byte;
    }
    (Scalar::from_bytes_reduced(&clamped), prefix)
}
