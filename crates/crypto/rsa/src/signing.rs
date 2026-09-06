// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Signing, which is not part of the product surface.
//!
//! A client that presents no certificate never signs. What is here exists
//! so that the test certificates of `audhsos-x509` are built and signed by
//! project code rather than vendored, and so that the negative tests of
//! this crate can make a signature over an encoded message that no
//! encoder would produce. It is one call into the exponentiation
//! verification already has, with the private exponent in place of the
//! public one, and no key is generated anywhere (D-83).

use crypto_bignum::MAX_BYTES;

use crate::error::RsaError;
use crate::hash::HashId;
use crate::key::PublicKey;
use crate::pkcs1;
use crate::window::head_mut;

/// The signature of an encoded message that is already the width of the
/// key: `EM` raised to the private exponent.
///
/// # Errors
///
/// [`RsaError::BadSignature`] when the encoded message is not the width
/// of the key or is not below the modulus.
pub fn sign_encoded(
    key: &PublicKey,
    private_exponent: &[u8],
    encoded: &[u8],
    signature: &mut [u8],
) -> Result<(), RsaError> {
    if encoded.len() != key.size() || signature.len() != key.size() {
        return Err(RsaError::BadSignature);
    }
    key.modulus()
        .pow_wide(encoded, private_exponent, signature)
        .map_err(|_| RsaError::BadSignature)
}

/// An RSASSA-PKCS1-v1_5 signature of `message`.
///
/// # Errors
///
/// As [`sign_encoded`], and when the key is too small for the encoding.
pub fn sign_pkcs1(
    key: &PublicKey,
    private_exponent: &[u8],
    hash: HashId,
    message: &[u8],
    signature: &mut [u8],
) -> Result<(), RsaError> {
    let mut encoded = [0u8; MAX_BYTES];
    pkcs1::encode(hash, message, head_mut(&mut encoded, key.size()))?;
    sign_encoded(
        key,
        private_exponent,
        head_mut(&mut encoded, key.size()),
        signature,
    )
}
