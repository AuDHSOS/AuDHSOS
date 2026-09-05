// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! ECDSA over P-256, in the shape certificates use it.
//!
//! The signature arrives as two integers; the DER that carries them is the
//! business of the certificate parser, not of this crate. The public key
//! arrives as the uncompressed point encoding of SEC 1, which is what an
//! X.509 subject public key holds for this curve.
//!
//! Verification handles public values only, so this module branches on
//! them freely. Signing exists for test data alone, is deterministic per
//! RFC 6979, and therefore needs no randomness.

pub mod field;
pub mod point;

use field::{Element, Order};
use point::Point;

use crate::error::EcError;

/// Bytes of an uncompressed public key.
pub const PUBLIC_LEN: usize = 65;
/// Bytes of a signature component.
pub const SCALAR_LEN: usize = 32;

/// The byte that introduces an uncompressed point.
const UNCOMPRESSED: u8 = 0x04;

/// An element of the scalar field.
type Scalar = Element<Order>;

/// A public key: a point of the curve that is not the neutral element.
#[derive(Clone, Copy, Debug)]
pub struct PublicKey {
    /// The point.
    point: Point,
}

impl PublicKey {
    /// The key the uncompressed encoding holds.
    ///
    /// # Errors
    ///
    /// [`EcError::InvalidPoint`] when the encoding is not sixty-five bytes
    /// introduced by `0x04`, when a coordinate is at or above the field
    /// prime, or when the point is not on the curve. Compressed encodings
    /// are refused as well: certificates for this curve carry the
    /// uncompressed form, and a path nothing exercises is a path nobody
    /// checks.
    pub fn from_sec1(bytes: &[u8]) -> Result<PublicKey, EcError> {
        let encoded: &[u8; PUBLIC_LEN] = bytes.try_into().map_err(|_| EcError::InvalidPoint)?;
        if encoded[0] != UNCOMPRESSED {
            return Err(EcError::InvalidPoint);
        }

        let (_, coordinates) = encoded.split_at(1);
        let (x_bytes, y_bytes) = coordinates.split_at(32);
        let mut x = [0u8; 32];
        let mut y = [0u8; 32];
        for (slot, byte) in x.iter_mut().zip(x_bytes) {
            *slot = *byte;
        }
        for (slot, byte) in y.iter_mut().zip(y_bytes) {
            *slot = *byte;
        }

        let x = Element::from_canonical(&x).ok_or(EcError::InvalidPoint)?;
        let y = Element::from_canonical(&y).ok_or(EcError::InvalidPoint)?;
        let point = Point::from_affine(x, y).ok_or(EcError::InvalidPoint)?;
        Ok(PublicKey { point })
    }

    /// Whether `(r, s)` is a signature of `digest` under this key.
    ///
    /// `digest` is the output of the hash the signature algorithm names.
    /// Only its leftmost bytes are used, as many as the group order is
    /// wide, which is what makes SHA-384 usable with this curve.
    ///
    /// # Errors
    ///
    /// [`EcError::InvalidScalar`] when `r` or `s` is zero or not below the
    /// group order; [`EcError::BadSignature`] when the equation does not
    /// hold.
    #[expect(
        clippy::many_single_char_names,
        reason = "the signature components are named as in the standard"
    )]
    pub fn verify(
        self,
        digest: &[u8],
        r: &[u8; SCALAR_LEN],
        s: &[u8; SCALAR_LEN],
    ) -> Result<(), EcError> {
        let r_value = Scalar::from_canonical(r).ok_or(EcError::InvalidScalar)?;
        let s_value = Scalar::from_canonical(s).ok_or(EcError::InvalidScalar)?;
        if r_value.is_zero() || s_value.is_zero() {
            return Err(EcError::InvalidScalar);
        }

        let e = scalar_of_digest(digest);
        let w = s_value.invert();
        let u1 = e.mul(w);
        let u2 = r_value.mul(w);

        let combined = Point::generator()
            .mul(&u1.to_bytes())
            .add(self.point.mul(&u2.to_bytes()));
        let (x, _) = combined.to_affine().ok_or(EcError::BadSignature)?;

        if Scalar::from_bytes_reduced(&x.to_bytes()) == r_value {
            Ok(())
        } else {
            Err(EcError::BadSignature)
        }
    }
}

/// The scalar a digest stands for: its leftmost thirty-two bytes, reduced.
fn scalar_of_digest(digest: &[u8]) -> Scalar {
    let mut bytes = [0u8; SCALAR_LEN];
    let taken = digest.len().min(SCALAR_LEN);
    let (leftmost, _) = digest.split_at(taken);
    let start = SCALAR_LEN.saturating_sub(taken);
    for (slot, byte) in bytes.iter_mut().skip(start).zip(leftmost) {
        *slot = *byte;
    }
    Scalar::from_bytes_reduced(&bytes)
}

/// The public key of `secret`.
///
/// # Errors
///
/// [`EcError::InvalidScalar`] when the secret is zero or not below the
/// group order.
#[cfg(any(test, feature = "test-signing"))]
pub fn public_key(secret: &[u8; SCALAR_LEN]) -> Result<[u8; PUBLIC_LEN], EcError> {
    let scalar = Scalar::from_canonical(secret).ok_or(EcError::InvalidScalar)?;
    if scalar.is_zero() {
        return Err(EcError::InvalidScalar);
    }
    let (x, y) = Point::generator()
        .mul(secret)
        .to_affine()
        .ok_or(EcError::InvalidScalar)?;

    let mut encoded = [0u8; PUBLIC_LEN];
    encoded[0] = UNCOMPRESSED;
    for (slot, byte) in encoded.iter_mut().skip(1).zip(x.to_bytes()) {
        *slot = byte;
    }
    for (slot, byte) in encoded.iter_mut().skip(33).zip(y.to_bytes()) {
        *slot = byte;
    }
    Ok(encoded)
}

/// The signature of `digest` under `secret`, with the nonce derived from
/// both as RFC 6979 prescribes.
///
/// Signing exists for the test data of this repository and is compiled
/// into product builds by no feature.
///
/// # Errors
///
/// [`EcError::InvalidScalar`] when the secret is zero or not below the
/// group order, or when the derivation finds no usable nonce, which needs
/// more failures in a row than the construction can produce.
#[cfg(any(test, feature = "test-signing"))]
pub fn sign(
    secret: &[u8; SCALAR_LEN],
    digest: &[u8; SCALAR_LEN],
) -> Result<([u8; SCALAR_LEN], [u8; SCALAR_LEN]), EcError> {
    use crypto_hash::{Hmac, Sha256};

    let x = Scalar::from_canonical(secret).ok_or(EcError::InvalidScalar)?;
    if x.is_zero() {
        return Err(EcError::InvalidScalar);
    }
    let e = scalar_of_digest(digest);

    // The generator of RFC 6979, section 3.2.
    let mut v = [0x01u8; 32];
    let mut k = [0x00u8; 32];
    for separator in [0x00u8, 0x01] {
        let mut mac = Hmac::<Sha256>::new(&k);
        mac.update(&v);
        mac.update(&[separator]);
        mac.update(secret);
        mac.update(&e.to_bytes());
        k = mac.finish();
        v = Hmac::<Sha256>::tag(&k, &v);
    }

    for _ in 0..64 {
        v = Hmac::<Sha256>::tag(&k, &v);
        if let Some(candidate) = Scalar::from_canonical(&v)
            && !candidate.is_zero()
            && let Some(signature) = attempt(candidate, x, e, &v)
        {
            return Ok(signature);
        }
        let mut mac = Hmac::<Sha256>::new(&k);
        mac.update(&v);
        mac.update(&[0x00]);
        k = mac.finish();
        v = Hmac::<Sha256>::tag(&k, &v);
    }
    Err(EcError::InvalidScalar)
}

/// One attempt at a signature with the nonce `nonce`, whose big-endian
/// encoding is `nonce_bytes`.
#[cfg(any(test, feature = "test-signing"))]
fn attempt(
    nonce: Scalar,
    secret: Scalar,
    digest: Scalar,
    nonce_bytes: &[u8; SCALAR_LEN],
) -> Option<([u8; SCALAR_LEN], [u8; SCALAR_LEN])> {
    let (x, _) = Point::generator().mul(nonce_bytes).to_affine()?;
    let r = Scalar::from_bytes_reduced(&x.to_bytes());
    if r.is_zero() {
        return None;
    }
    let s = nonce.invert().mul(digest.add(r.mul(secret)));
    if s.is_zero() {
        return None;
    }
    Some((r.to_bytes(), s.to_bytes()))
}
