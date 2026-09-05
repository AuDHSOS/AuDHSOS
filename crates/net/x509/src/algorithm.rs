// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The algorithms a certificate names, and what verifying with them means.

use audhsos_der::{Reader, Tag};
use crypto_ec::ed25519;
use crypto_ec::p256::PublicKey as P256Key;
use crypto_ec::p384::PublicKey as P384Key;
use crypto_hash::{Sha256, Sha384};

use crate::error::X509Error;
use crate::oid;

/// A signature algorithm this crate verifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignatureAlgorithm {
    /// ECDSA over P-256 with SHA-256.
    EcdsaSha256,
    /// ECDSA over P-256 with SHA-384.
    EcdsaSha384,
    /// Ed25519, which hashes the body itself.
    Ed25519,
}

impl SignatureAlgorithm {
    /// The algorithm the identifier names.
    ///
    /// # Errors
    ///
    /// [`X509Error::UnsupportedAlgorithm`] for any other identifier, and
    /// [`X509Error::BadExtension`] when the parameters are not what the
    /// algorithm prescribes: absent for both of these, since ECDSA
    /// signature algorithms take none and Ed25519 forbids them.
    pub fn parse(reader: &mut Reader<'_>) -> Result<SignatureAlgorithm, X509Error> {
        let mut fields = reader.read_sequence()?;
        let identifier = fields.read_object_identifier()?;
        let algorithm = match identifier.as_bytes() {
            oid::ECDSA_WITH_SHA256 => SignatureAlgorithm::EcdsaSha256,
            oid::ECDSA_WITH_SHA384 => SignatureAlgorithm::EcdsaSha384,
            oid::ED25519 => SignatureAlgorithm::Ed25519,
            _ => return Err(X509Error::UnsupportedAlgorithm),
        };
        fields.finish()?;
        Ok(algorithm)
    }
}

/// A public key a certificate carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubjectPublicKey<'a> {
    /// A point of P-256, in the uncompressed encoding.
    EcdsaP256(&'a [u8]),
    /// A point of P-384, in the uncompressed encoding.
    EcdsaP384(&'a [u8]),
    /// An Ed25519 key.
    Ed25519(&'a [u8]),
}

impl<'a> SubjectPublicKey<'a> {
    /// The key the subject public key information holds.
    ///
    /// # Errors
    ///
    /// [`X509Error::UnsupportedAlgorithm`] for another algorithm or
    /// another curve, [`X509Error::BadPublicKey`] when the key does not
    /// have the length its algorithm prescribes or the bit string does not
    /// end on a byte.
    pub fn parse(reader: &mut Reader<'a>) -> Result<SubjectPublicKey<'a>, X509Error> {
        let mut info = reader.read_sequence()?;
        let mut algorithm = info.read_sequence()?;
        let identifier = algorithm.read_object_identifier()?;

        let key = match identifier.as_bytes() {
            oid::EC_PUBLIC_KEY => {
                // RFC 5480, section 2.2: the point is the uncompressed
                // encoding, one byte of form and two coordinates, so its
                // width says which curve it is as surely as the identifier
                // does. Both are checked, and they must agree.
                let curve = algorithm.read_object_identifier()?;
                let width = match curve.as_bytes() {
                    oid::PRIME256V1 => 65,
                    oid::SECP384R1 => 97,
                    _ => return Err(X509Error::UnsupportedAlgorithm),
                };
                algorithm.finish()?;
                let bits = info.read_bit_string()?.whole_bytes()?;
                if bits.len() != width {
                    return Err(X509Error::BadPublicKey);
                }
                if width == 65 {
                    SubjectPublicKey::EcdsaP256(bits)
                } else {
                    SubjectPublicKey::EcdsaP384(bits)
                }
            }
            oid::ED25519 => {
                // RFC 8410: the parameters field is absent, not null.
                algorithm.finish()?;
                let bits = info.read_bit_string()?.whole_bytes()?;
                if bits.len() != 32 {
                    return Err(X509Error::BadPublicKey);
                }
                SubjectPublicKey::Ed25519(bits)
            }
            _ => return Err(X509Error::UnsupportedAlgorithm),
        };
        info.finish()?;
        Ok(key)
    }

    /// Whether `signature` is a signature of `body` under this key and
    /// `algorithm`.
    ///
    /// # Errors
    ///
    /// [`X509Error::UnsupportedAlgorithm`] when the key and the algorithm
    /// do not belong together, [`X509Error::BadSignature`] when the
    /// signature does not have the shape the algorithm prescribes, and
    /// [`X509Error::SignatureFailed`] when it simply does not verify.
    pub fn verify(
        self,
        algorithm: SignatureAlgorithm,
        body: &[u8],
        signature: &[u8],
    ) -> Result<(), X509Error> {
        match (self, algorithm) {
            (SubjectPublicKey::Ed25519(key), SignatureAlgorithm::Ed25519) => {
                let key: &[u8; 32] = key.try_into().map_err(|_| X509Error::BadPublicKey)?;
                let signature: &[u8; 64] =
                    signature.try_into().map_err(|_| X509Error::BadSignature)?;
                ed25519::verify(key, body, signature).map_err(|_| X509Error::SignatureFailed)
            }
            (SubjectPublicKey::EcdsaP256(key), SignatureAlgorithm::EcdsaSha256) => {
                let digest = Sha256::digest(body);
                verify_ecdsa(key, digest.as_ref(), signature)
            }
            (SubjectPublicKey::EcdsaP256(key), SignatureAlgorithm::EcdsaSha384) => {
                let digest = Sha384::digest(body);
                verify_ecdsa(key, digest.as_ref(), signature)
            }
            (SubjectPublicKey::EcdsaP384(key), SignatureAlgorithm::EcdsaSha256) => {
                let digest = Sha256::digest(body);
                verify_ecdsa_p384(key, digest.as_ref(), signature)
            }
            (SubjectPublicKey::EcdsaP384(key), SignatureAlgorithm::EcdsaSha384) => {
                let digest = Sha384::digest(body);
                verify_ecdsa_p384(key, digest.as_ref(), signature)
            }
            _ => Err(X509Error::UnsupportedAlgorithm),
        }
    }
}

/// Verifies an ECDSA signature, whose two integers arrive wrapped in a
/// sequence of their own.
fn verify_ecdsa(key: &[u8], digest: &[u8], signature: &[u8]) -> Result<(), X509Error> {
    let key = P256Key::from_sec1(key).map_err(|_| X509Error::BadPublicKey)?;
    let (r, s) = signature_pair(signature)?;
    key.verify(digest, &right_aligned(r)?, &right_aligned(s)?)
        .map_err(|_| X509Error::SignatureFailed)
}

/// The same over P-384.
fn verify_ecdsa_p384(key: &[u8], digest: &[u8], signature: &[u8]) -> Result<(), X509Error> {
    let key = P384Key::from_sec1(key).map_err(|_| X509Error::BadPublicKey)?;
    let (r, s) = signature_pair(signature)?;
    key.verify(digest, &right_aligned(r)?, &right_aligned(s)?)
        .map_err(|_| X509Error::SignatureFailed)
}

/// The two integers of a DER `ECDSA-Sig-Value`, as RFC 5480 defines it.
fn signature_pair(signature: &[u8]) -> Result<(&[u8], &[u8]), X509Error> {
    let mut outer = Reader::new(signature);
    let mut pair = outer.read_sequence().map_err(|_| X509Error::BadSignature)?;
    let r = pair.read_integer().map_err(|_| X509Error::BadSignature)?;
    let s = pair.read_integer().map_err(|_| X509Error::BadSignature)?;
    pair.finish().map_err(|_| X509Error::BadSignature)?;
    outer.finish().map_err(|_| X509Error::BadSignature)?;
    Ok((r, s))
}

/// A signature component as `N` bytes, aligned to the right. DER drops
/// the leading zeros of an integer, and the curve decides how many go
/// back on.
fn right_aligned<const N: usize>(value: &[u8]) -> Result<[u8; N], X509Error> {
    let start = N.checked_sub(value.len()).ok_or(X509Error::BadSignature)?;
    let mut bytes = [0u8; N];
    for (slot, byte) in bytes.iter_mut().skip(start).zip(value) {
        *slot = *byte;
    }
    Ok(bytes)
}

/// Whether the next value is an algorithm identifier equal to `expected`,
/// consuming it either way.
///
/// # Errors
///
/// The errors of [`SignatureAlgorithm::parse`], and
/// [`X509Error::AlgorithmMismatch`] when the two differ.
pub fn expect_same(reader: &mut Reader<'_>, expected: SignatureAlgorithm) -> Result<(), X509Error> {
    if SignatureAlgorithm::parse(reader)? == expected {
        Ok(())
    } else {
        Err(X509Error::AlgorithmMismatch)
    }
}

/// The tag of a `dNSName` inside a subject alternative name: context two,
/// primitive, because the string is tagged implicitly.
pub const DNS_NAME_TAG: Tag = Tag::context(2, false);
