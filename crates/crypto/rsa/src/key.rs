// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The public key, and the two verifications over it.

use crypto_bignum::{MAX_BYTES, Modulus};
use crypto_ct::ct_eq;

use crate::error::RsaError;
use crate::hash::HashId;
use crate::pkcs1;
use crate::pss;
use crate::window::{head, head_mut};

/// An RSA public key: a modulus, a public exponent, and the width the
/// pair encodes and verifies at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicKey {
    /// The modulus, with the constants Montgomery arithmetic reduces by.
    modulus: Modulus,
    /// The public exponent.
    exponent: u64,
    /// Bytes of the modulus, which is `k` in RFC 8017 and the length
    /// every signature under this key has.
    size: usize,
}

impl PublicKey {
    /// The key the two big-endian encodings carry.
    ///
    /// These are the shape rules that belong to the primitive: the
    /// modulus odd with its top bit set and no wider than the arithmetic
    /// allows, the exponent odd and at least three. The lower bound on
    /// the key size is not here — it is a rule about certificates, and it
    /// lives where certificates are judged (D-79).
    ///
    /// # Errors
    ///
    /// [`RsaError::InvalidModulus`] and [`RsaError::InvalidExponent`] for
    /// the halves that fail.
    pub fn new(modulus: &[u8], exponent: &[u8]) -> Result<PublicKey, RsaError> {
        if modulus.first().copied().unwrap_or(0) & 0x80 == 0 {
            return Err(RsaError::InvalidModulus);
        }
        let value = Modulus::new(modulus).map_err(|_| RsaError::InvalidModulus)?;
        Ok(PublicKey {
            modulus: value,
            exponent: exponent_of(exponent)?,
            size: modulus.len(),
        })
    }

    /// Bytes of the modulus, which is the length of every signature under
    /// this key.
    #[must_use]
    pub const fn size(&self) -> usize {
        self.size
    }

    /// Bits of the modulus, which is `modBits` in RFC 8017.
    #[must_use]
    pub fn bits(&self) -> usize {
        self.modulus.bits()
    }

    /// The public exponent.
    #[must_use]
    pub const fn exponent(&self) -> u64 {
        self.exponent
    }

    /// Whether `signature` is an RSASSA-PKCS1-v1_5 signature of `message`
    /// under this key, by RFC 8017, section 8.2.2.
    ///
    /// # Errors
    ///
    /// [`RsaError::BadSignature`] for every way it can fail to be one.
    pub fn verify_pkcs1(
        &self,
        hash: HashId,
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), RsaError> {
        let mut recovered = [0u8; MAX_BYTES];
        self.recover(signature, &mut recovered)?;
        let mut expected = [0u8; MAX_BYTES];
        pkcs1::encode(hash, message, head_mut(&mut expected, self.size))?;
        if ct_eq(head(&recovered, self.size), head(&expected, self.size)).is_true() {
            return Ok(());
        }
        Err(RsaError::BadSignature)
    }

    /// Whether `signature` is an RSASSA-PSS signature of `message` under
    /// this key, by RFC 8017, section 8.1.2.
    ///
    /// The encoded message is `modBits - 1` bits wide, which for a
    /// modulus with its top bit set is one bit short of the whole key, so
    /// the leftmost bit of the encoding is the one that must be zero.
    ///
    /// # Errors
    ///
    /// [`RsaError::BadSignature`] for every way it can fail to be one.
    pub fn verify_pss(
        &self,
        hash: HashId,
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), RsaError> {
        let mut recovered = [0u8; MAX_BYTES];
        self.recover(signature, &mut recovered)?;
        let em_bits = self.bits().saturating_sub(1);
        pss::verify(hash, em_bits, message, head(&recovered, self.size))
    }

    /// The RSAVP1 of RFC 8017, section 5.2.2: the signature raised to the
    /// public exponent, written into the front of `out`.
    pub(crate) fn recover(
        &self,
        signature: &[u8],
        out: &mut [u8; MAX_BYTES],
    ) -> Result<(), RsaError> {
        if signature.len() != self.size {
            return Err(RsaError::BadSignature);
        }
        self.modulus
            .pow(signature, self.exponent, head_mut(out, self.size))
            .map_err(|_| RsaError::BadSignature)
    }

    /// The modulus, for the one caller that has a private exponent and
    /// wants the same arithmetic.
    #[cfg(feature = "test-signing")]
    pub(crate) const fn modulus(&self) -> &Modulus {
        &self.modulus
    }
}

/// The exponent a big-endian encoding carries, checked against the rules
/// of D-79 that belong to the primitive.
fn exponent_of(bytes: &[u8]) -> Result<u64, RsaError> {
    let mut value = 0u64;
    for byte in bytes {
        value = value
            .checked_mul(256)
            .and_then(|shifted| shifted.checked_add(u64::from(*byte)))
            .ok_or(RsaError::InvalidExponent)?;
    }
    if value < 3 || value & 1 == 0 {
        return Err(RsaError::InvalidExponent);
    }
    Ok(value)
}
