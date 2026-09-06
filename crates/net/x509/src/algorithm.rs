// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The algorithms a certificate names, and what verifying with them means.

use audhsos_der::{Reader, Tag};
use crypto_ec::ed25519;
use crypto_ec::p256::PublicKey as P256Key;
use crypto_ec::p384::PublicKey as P384Key;
use crypto_hash::{Sha256, Sha384};
use crypto_rsa::{HashId, PublicKey as RsaKey};

use crate::error::X509Error;
use crate::oid;

/// The narrowest RSA key this system accepts from a certificate, in bits.
///
/// D-79 puts the lower bound here and not in `crypto-rsa`: the primitive
/// verifies what it is given, and the layer that judges a certificate is
/// the layer that decides a key is too small. That separation is what
/// lets the thousand-and-twenty-four-bit key of RFC 8448 exercise the
/// primitive while no chain with such a key is ever walked.
pub const MIN_RSA_BITS: usize = 2048;

/// The widest RSA key this system accepts, in bits, which is what the
/// arithmetic of `crypto-bignum` can represent (D-78).
pub const MAX_RSA_BITS: usize = 4096;

/// A signature algorithm this crate verifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignatureAlgorithm {
    /// ECDSA over P-256 with SHA-256.
    EcdsaSha256,
    /// ECDSA over P-256 with SHA-384.
    EcdsaSha384,
    /// Ed25519, which hashes the body itself.
    Ed25519,
    /// RSASSA-PKCS1-v1_5 with SHA-256, `sha256WithRSAEncryption`.
    RsaPkcs1Sha256,
    /// RSASSA-PKCS1-v1_5 with SHA-384, `sha384WithRSAEncryption`.
    RsaPkcs1Sha384,
    /// RSASSA-PKCS1-v1_5 with SHA-512, `sha512WithRSAEncryption`.
    RsaPkcs1Sha512,
    /// RSASSA-PSS with SHA-256, MGF1 over SHA-256, and a salt of
    /// thirty-two bytes.
    RsaPssSha256,
    /// RSASSA-PSS with SHA-384, MGF1 over SHA-384, and a salt of
    /// forty-eight bytes.
    RsaPssSha384,
    /// RSASSA-PSS with SHA-512, MGF1 over SHA-512, and a salt of
    /// sixty-four bytes.
    RsaPssSha512,
}

impl SignatureAlgorithm {
    /// The algorithm the identifier names.
    ///
    /// There is a rule per algorithm here and not one rule for all of
    /// them, which is what the three RSA identifiers of RFC 4055,
    /// section 5 cost. Their parameters must be NULL, and an
    /// implementation must accept them absent as well; ECDSA and Ed25519
    /// allow absence and nothing else. `id-RSASSA-PSS` is the third rule
    /// again: its parameters are required, and only the three sets that
    /// pair a hash with MGF1 over that same hash and a salt as long as
    /// its output are read (D-81).
    ///
    /// # Errors
    ///
    /// [`X509Error::UnsupportedAlgorithm`] for any other identifier, for
    /// a PSS parameter set that is not one of the three, and — through
    /// `finish` — for parameters the algorithm does not allow.
    pub fn parse(reader: &mut Reader<'_>) -> Result<SignatureAlgorithm, X509Error> {
        let mut fields = reader.read_sequence()?;
        let identifier = fields.read_object_identifier()?;
        let algorithm = match identifier.as_bytes() {
            // RFC 5758, section 3.2 and RFC 8410, section 3: the
            // parameters field is absent, which `finish` below is the
            // whole of.
            oid::ECDSA_WITH_SHA256 => SignatureAlgorithm::EcdsaSha256,
            oid::ECDSA_WITH_SHA384 => SignatureAlgorithm::EcdsaSha384,
            oid::ED25519 => SignatureAlgorithm::Ed25519,
            // RFC 4055, section 5: NULL, and absence accepted as well.
            oid::SHA256_WITH_RSA => {
                take_null(&mut fields)?;
                SignatureAlgorithm::RsaPkcs1Sha256
            }
            oid::SHA384_WITH_RSA => {
                take_null(&mut fields)?;
                SignatureAlgorithm::RsaPkcs1Sha384
            }
            oid::SHA512_WITH_RSA => {
                take_null(&mut fields)?;
                SignatureAlgorithm::RsaPkcs1Sha512
            }
            oid::RSASSA_PSS => read_pss_parameters(&mut fields)?,
            _ => return Err(X509Error::UnsupportedAlgorithm),
        };
        fields.finish()?;
        Ok(algorithm)
    }

    /// The hash this algorithm signs with, for the RSA schemes.
    const fn rsa_hash(self) -> Option<HashId> {
        match self {
            SignatureAlgorithm::RsaPkcs1Sha256 | SignatureAlgorithm::RsaPssSha256 => {
                Some(HashId::Sha256)
            }
            SignatureAlgorithm::RsaPkcs1Sha384 | SignatureAlgorithm::RsaPssSha384 => {
                Some(HashId::Sha384)
            }
            SignatureAlgorithm::RsaPkcs1Sha512 | SignatureAlgorithm::RsaPssSha512 => {
                Some(HashId::Sha512)
            }
            _ => None,
        }
    }

    /// Whether this algorithm is one of the three PSS schemes.
    const fn is_pss(self) -> bool {
        matches!(
            self,
            SignatureAlgorithm::RsaPssSha256
                | SignatureAlgorithm::RsaPssSha384
                | SignatureAlgorithm::RsaPssSha512
        )
    }
}

/// Consumes a NULL if one is there, which is where RFC 4055, section 5
/// differs from every other algorithm this crate reads.
fn take_null(fields: &mut Reader<'_>) -> Result<(), X509Error> {
    if fields.peek_is(Tag::NULL) {
        fields.read_null()?;
    }
    Ok(())
}

/// The `RSASSA-PSS-params` of RFC 4055, section 3.1, restricted to the
/// three parameter sets its section 6 pairs with the `rsae` schemes.
///
/// Exactly three fields are present and no more. `trailerField` is `1`,
/// which is its default, so the distinguished rules leave it out; the
/// other three all differ from their defaults — SHA-1, MGF1 over SHA-1,
/// and a salt of twenty — so none of them may be left out.
fn read_pss_parameters(fields: &mut Reader<'_>) -> Result<SignatureAlgorithm, X509Error> {
    let mut params = fields.read_sequence()?;

    let mut hash_field = params.read_context(0)?;
    let hash = read_hash_identifier(&mut hash_field)?;
    hash_field.finish()?;

    let mut mask_field = params.read_context(1)?;
    let mut mask = mask_field.read_sequence()?;
    if mask.read_object_identifier()?.as_bytes() != oid::MGF1 {
        return Err(X509Error::UnsupportedAlgorithm);
    }
    let mask_hash = read_hash_identifier(&mut mask)?;
    mask.finish()?;
    mask_field.finish()?;

    let mut salt_field = params.read_context(2)?;
    let salt = salt_field.read_integer()?;
    salt_field.finish()?;
    params.finish()?;

    if mask_hash != hash {
        return Err(X509Error::UnsupportedAlgorithm);
    }
    let expected = u8::try_from(hash.output_len()).unwrap_or(0);
    if salt != [expected].as_slice() {
        return Err(X509Error::UnsupportedAlgorithm);
    }
    Ok(match hash {
        HashId::Sha256 => SignatureAlgorithm::RsaPssSha256,
        HashId::Sha384 => SignatureAlgorithm::RsaPssSha384,
        HashId::Sha512 => SignatureAlgorithm::RsaPssSha512,
    })
}

/// One `HashAlgorithm` of RFC 4055, section 2.1, whose parameters may be
/// NULL or absent and where both must be accepted.
fn read_hash_identifier(reader: &mut Reader<'_>) -> Result<HashId, X509Error> {
    let mut fields = reader.read_sequence()?;
    let identifier = fields.read_object_identifier()?;
    let hash = match identifier.as_bytes() {
        oid::SHA256 => HashId::Sha256,
        oid::SHA384 => HashId::Sha384,
        oid::SHA512 => HashId::Sha512,
        _ => return Err(X509Error::UnsupportedAlgorithm),
    };
    take_null(&mut fields)?;
    fields.finish()?;
    Ok(hash)
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
    /// An RSA key: the two integers of the `RSAPublicKey` of RFC 3279,
    /// section 2.3.1, each without the leading zero the encoding puts on
    /// a positive value.
    Rsa {
        /// The modulus.
        modulus: &'a [u8],
        /// The public exponent.
        exponent: &'a [u8],
    },
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
            oid::RSA_ENCRYPTION => {
                // RFC 3279, section 2.3.1: the parameters field MUST have
                // ASN.1 type NULL for this identifier, and the key is the
                // DER encoding of `RSAPublicKey ::= SEQUENCE { modulus
                // INTEGER, publicExponent INTEGER }`. The reader needs
                // nothing new for it: `read_integer` already strips the
                // leading zero of a positive value and refuses a negative
                // one.
                algorithm.read_null()?;
                algorithm.finish()?;
                let bits = info.read_bit_string()?.whole_bytes()?;
                let mut outer = Reader::new(bits);
                let mut pair = outer.read_sequence()?;
                let modulus = pair.read_integer()?;
                let exponent = pair.read_integer()?;
                pair.finish()?;
                outer.finish()?;
                SubjectPublicKey::Rsa { modulus, exponent }
            }
            _ => return Err(X509Error::UnsupportedAlgorithm),
        };
        info.finish()?;
        Ok(key)
    }

    /// Whether this is a key the verification of this crate can use.
    ///
    /// Parsing establishes that the algorithm is one of the three and that
    /// the encoding has the width that algorithm prescribes. This goes one
    /// step further for the two curves and asks whether the coordinates
    /// are a point of the curve, which is what a verification would need
    /// them to be. Ed25519 has no equivalent step that is cheaper than the
    /// verification itself, so for that key the width is all there is.
    ///
    /// It exists for [`crate::path::TrustAnchor::from_certificate`], where
    /// a key that cannot be used is worth refusing while the caller still
    /// holds the file it came from.
    ///
    /// # Errors
    ///
    /// [`X509Error::BadPublicKey`] when the key is not one of its kind.
    pub(crate) fn check_usable(self) -> Result<(), X509Error> {
        match self {
            SubjectPublicKey::EcdsaP256(key) => P256Key::from_sec1(key)
                .map(|_| ())
                .map_err(|_| X509Error::BadPublicKey),
            SubjectPublicKey::EcdsaP384(key) => P384Key::from_sec1(key)
                .map(|_| ())
                .map_err(|_| X509Error::BadPublicKey),
            SubjectPublicKey::Ed25519(key) => {
                if key.len() == 32 {
                    Ok(())
                } else {
                    Err(X509Error::BadPublicKey)
                }
            }
            // The size bound of D-79, which is what makes a key out of
            // range a refusal rather than a path that reaches nothing.
            SubjectPublicKey::Rsa { modulus, exponent } => {
                let key = RsaKey::new(modulus, exponent).map_err(|_| X509Error::BadPublicKey)?;
                if (MIN_RSA_BITS..=MAX_RSA_BITS).contains(&key.bits()) {
                    Ok(())
                } else {
                    Err(X509Error::BadPublicKey)
                }
            }
        }
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
            (SubjectPublicKey::Rsa { modulus, exponent }, _) => {
                verify_rsa(modulus, exponent, algorithm, body, signature)
            }
            _ => Err(X509Error::UnsupportedAlgorithm),
        }
    }
}

/// Verifies an RSA signature under whichever of the six schemes the
/// certificate named.
fn verify_rsa(
    modulus: &[u8],
    exponent: &[u8],
    algorithm: SignatureAlgorithm,
    body: &[u8],
    signature: &[u8],
) -> Result<(), X509Error> {
    let hash = algorithm
        .rsa_hash()
        .ok_or(X509Error::UnsupportedAlgorithm)?;
    let key = RsaKey::new(modulus, exponent).map_err(|_| X509Error::BadPublicKey)?;
    let outcome = if algorithm.is_pss() {
        key.verify_pss(hash, body, signature)
    } else {
        key.verify_pkcs1(hash, body, signature)
    };
    outcome.map_err(|_| X509Error::SignatureFailed)
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
