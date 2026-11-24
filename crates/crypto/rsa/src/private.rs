// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The private key, and the PKCS #1 v1.5 signature it makes (D-197).
//!
//! Signing runs on `Modulus::pow_secret`, the Montgomery ladder of
//! `crypto-bignum`: one squaring and one multiplication per bit of the
//! exponent buffer, whatever the bits are. The exponent is held at the
//! width of the modulus, so the ladder length is the key size and not the
//! length of the exponent.

use crypto_bignum::MAX_BYTES;
use crypto_ct::wipe;

use crate::error::RsaError;
use crate::hash::HashId;
use crate::key::PublicKey;
use crate::pkcs1;
use crate::window::{head, head_mut, tail, tail_mut};

/// An RSA private key: a public key and its private exponent.
///
/// `Drop` wipes the exponent. The type carries no `Clone` and no `Debug`,
/// so the exponent has one copy and reaches no formatter.
///
/// ```compile_fail
/// use crypto_rsa::PrivateKey;
/// fn copied(key: &PrivateKey) -> PrivateKey {
///     key.clone()
/// }
/// ```
pub struct PrivateKey {
    /// The public half.
    public: PublicKey,
    /// The private exponent, big-endian, left-padded to the key size.
    exponent: [u8; MAX_BYTES],
}

impl PrivateKey {
    /// The key of `public` and the big-endian private exponent.
    ///
    /// Leading zero bytes of `private_exponent` are dropped, so the sign
    /// byte of a DER `INTEGER` is accepted. Whether the exponent belongs
    /// to the modulus is checked by [`PrivateKey::sign_pkcs1`], which
    /// verifies every signature it makes.
    ///
    /// # Errors
    ///
    /// [`RsaError::InvalidExponent`] when the exponent is zero or wider
    /// than the modulus.
    pub fn new(public: PublicKey, private_exponent: &[u8]) -> Result<PrivateKey, RsaError> {
        let start = private_exponent
            .iter()
            .position(|byte| *byte != 0)
            .ok_or(RsaError::InvalidExponent)?;
        let digits = tail(private_exponent, start);
        let size = public.size();
        if digits.len() > size {
            return Err(RsaError::InvalidExponent);
        }
        let mut exponent = [0u8; MAX_BYTES];
        tail_mut(
            head_mut(&mut exponent, size),
            size.saturating_sub(digits.len()),
        )
        .copy_from_slice(digits);
        Ok(PrivateKey { public, exponent })
    }

    /// The public half of this key.
    #[must_use]
    pub const fn public(&self) -> &PublicKey {
        &self.public
    }

    /// An RSASSA-PKCS1-v1_5 signature of `message`, by RFC 8017,
    /// section 8.2.1.
    ///
    /// The signature is verified under the public key before it is
    /// returned, so a fault in the exponentiation or an exponent of
    /// another key yields an error and not a wrong signature.
    ///
    /// # Errors
    ///
    /// [`RsaError::BadSignature`] when `signature` is not the key size,
    /// when the hash does not fit the key, or when the signature does not
    /// verify. `signature` is all zeros after an error.
    pub fn sign_pkcs1(
        &self,
        hash: HashId,
        message: &[u8],
        signature: &mut [u8],
    ) -> Result<(), RsaError> {
        let size = self.public.size();
        let mut encoded = [0u8; MAX_BYTES];
        let mut power = [0u8; MAX_BYTES];
        let modulus = self.public.modulus();
        let width = modulus.width();
        let computed = signature.len() == size
            && pkcs1::encode(hash, message, head_mut(&mut encoded, size)).is_ok()
            && modulus
                .pow_secret(
                    head(&encoded, size),
                    head(&self.exponent, size),
                    head_mut(&mut power, width),
                )
                .is_ok();
        if computed {
            signature.copy_from_slice(tail(head(&power, width), width.saturating_sub(size)));
        }
        wipe(&mut power);
        if computed && self.public.verify_pkcs1(hash, message, signature).is_ok() {
            return Ok(());
        }
        wipe(signature);
        Err(RsaError::BadSignature)
    }
}

impl Drop for PrivateKey {
    fn drop(&mut self) {
        wipe(&mut self.exponent);
    }
}
