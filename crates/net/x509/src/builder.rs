// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A writer for the certificates the tests need.
//!
//! It exists so that no certificate in this repository was made elsewhere.
//! Every chain the tests use — expired, wrongly named, badly signed,
//! missing a constraint — is built here and signed with the deterministic
//! signing of `crypto-ec`, so the private key of every test certificate is
//! a constant in a test file and has never been anywhere else.
//!
//! The writer is not a general encoder. It writes the shapes this crate
//! parses, into a buffer the caller owns, and refuses anything that does
//! not fit.

use audhsos_time::CivilTime;
use crypto_ec::{ed25519, p256, p384};
use crypto_hash::{Sha256, Sha384};
use crypto_rsa::signing::{sign_pkcs1, sign_pss};
use crypto_rsa::{HashId, PublicKey as RsaKey};

use crate::algorithm::SignatureAlgorithm;
use crate::certificate::BasicConstraints;
use crate::error::X509Error;
use crate::oid;

/// The largest certificate the builder writes.
///
/// A four-thousand-and-ninety-six-bit leaf under an issuer of the same
/// width is about fourteen hundred and fifty bytes, which is what took
/// this past the thousand and twenty-four it stood at while every key
/// here was a curve point.
pub const MAX_CERTIFICATE: usize = 2048;

/// Bytes a name, a validity, or one extension is built in.
const PART: usize = 256;

/// Bytes the body of the widest RSA public key occupies: the `SEQUENCE`
/// of two integers of RFC 3279, section 2.3.1, at four thousand and
/// ninety-six bits. Five hundred and seventeen for the modulus, five for
/// the exponent, four for the sequence around them.
pub const MAX_RSA_KEY: usize = 526;

/// Bytes the widest signature this builder writes occupies, which is an
/// RSA signature at the widest key.
const MAX_SIGNATURE: usize = 512;

/// Bytes the widest subject public key information occupies: fifteen for
/// the algorithm identifier, five hundred and thirty-one for the bit
/// string around the key body, four for the sequence around both.
pub const MAX_SPKI: usize = 550;

/// The public exponent every RSA test key uses, sixty-five thousand five
/// hundred and thirty-seven.
pub const PUBLIC_EXPONENT: &[u8] = &[0x01, 0x00, 0x01];

/// An RSA key pair as the tests hold it.
///
/// The two halves are borrowed rather than copied: a modulus and a
/// private exponent are half a kibibyte each at the widest, and
/// [`TestKey`] is passed by value through every builder call. The public
/// exponent is [`PUBLIC_EXPONENT`] for every one of them.
///
/// No project code generates an RSA key (D-83). The pairs these point at
/// are constants in test sources, each recorded with where it came from.
#[derive(Clone, Copy, Debug)]
pub struct RsaTestKey {
    /// The modulus, big-endian, without a leading zero.
    pub modulus: &'static [u8],
    /// The private exponent, big-endian.
    pub private_exponent: &'static [u8],
}

/// Which of the six RSA schemes a key signs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RsaScheme {
    /// PKCS #1 v1.5 with SHA-256.
    Pkcs1Sha256,
    /// PKCS #1 v1.5 with SHA-384.
    Pkcs1Sha384,
    /// PKCS #1 v1.5 with SHA-512.
    Pkcs1Sha512,
    /// PSS with SHA-256.
    PssSha256,
    /// PSS with SHA-384.
    PssSha384,
    /// PSS with SHA-512.
    PssSha512,
}

impl RsaScheme {
    /// The algorithm a certificate names for this scheme.
    #[must_use]
    pub const fn algorithm(self) -> SignatureAlgorithm {
        match self {
            RsaScheme::Pkcs1Sha256 => SignatureAlgorithm::RsaPkcs1Sha256,
            RsaScheme::Pkcs1Sha384 => SignatureAlgorithm::RsaPkcs1Sha384,
            RsaScheme::Pkcs1Sha512 => SignatureAlgorithm::RsaPkcs1Sha512,
            RsaScheme::PssSha256 => SignatureAlgorithm::RsaPssSha256,
            RsaScheme::PssSha384 => SignatureAlgorithm::RsaPssSha384,
            RsaScheme::PssSha512 => SignatureAlgorithm::RsaPssSha512,
        }
    }

    /// The hash it signs with.
    const fn hash(self) -> HashId {
        match self {
            RsaScheme::Pkcs1Sha256 | RsaScheme::PssSha256 => HashId::Sha256,
            RsaScheme::Pkcs1Sha384 | RsaScheme::PssSha384 => HashId::Sha384,
            RsaScheme::Pkcs1Sha512 | RsaScheme::PssSha512 => HashId::Sha512,
        }
    }

    /// Whether it is one of the three PSS schemes.
    const fn is_pss(self) -> bool {
        matches!(
            self,
            RsaScheme::PssSha256 | RsaScheme::PssSha384 | RsaScheme::PssSha512
        )
    }
}

/// A key that signs, and the algorithm it signs with.
#[derive(Clone, Copy, Debug)]
pub enum TestKey {
    /// An Ed25519 key.
    Ed25519([u8; 32]),
    /// A P-256 key signing with SHA-256.
    EcdsaSha256([u8; 32]),
    /// A P-256 key signing with SHA-384. X.509 allows the pair; a TLS
    /// signature scheme does not, which is what makes this key useful to
    /// the test that checks the difference.
    EcdsaSha384([u8; 32]),
    /// A P-384 key signing with SHA-384.
    EcdsaP384Sha384([u8; 48]),
    /// An RSA key signing with one of the six schemes.
    Rsa(RsaTestKey, RsaScheme),
}

impl TestKey {
    /// The algorithm this key signs with.
    #[must_use]
    pub const fn algorithm(self) -> SignatureAlgorithm {
        match self {
            TestKey::Ed25519(_) => SignatureAlgorithm::Ed25519,
            TestKey::EcdsaSha256(_) => SignatureAlgorithm::EcdsaSha256,
            TestKey::EcdsaSha384(_) | TestKey::EcdsaP384Sha384(_) => {
                SignatureAlgorithm::EcdsaSha384
            }
            TestKey::Rsa(_, scheme) => scheme.algorithm(),
        }
    }

    /// The public key, and how many bytes of the array it occupies.
    ///
    /// # Errors
    ///
    /// [`X509Error::BadPublicKey`] when the secret is out of range, which
    /// only a hand-written test key can be.
    pub fn public_key(self) -> Result<([u8; MAX_RSA_KEY], usize), X509Error> {
        let mut bytes = [0u8; MAX_RSA_KEY];
        match self {
            TestKey::Ed25519(secret) => {
                let public = ed25519::public_key(&secret);
                for (slot, byte) in bytes.iter_mut().zip(public) {
                    *slot = byte;
                }
                Ok((bytes, 32))
            }
            TestKey::EcdsaSha256(secret) | TestKey::EcdsaSha384(secret) => {
                let public = p256::public_key(&secret).map_err(|_| X509Error::BadPublicKey)?;
                for (slot, byte) in bytes.iter_mut().zip(public) {
                    *slot = byte;
                }
                Ok((bytes, 65))
            }
            TestKey::EcdsaP384Sha384(secret) => {
                let public = p384::public_key(&secret).map_err(|_| X509Error::BadPublicKey)?;
                for (slot, byte) in bytes.iter_mut().zip(public) {
                    *slot = byte;
                }
                Ok((bytes, 97))
            }
            TestKey::Rsa(key, _) => {
                // RFC 3279, section 2.3.1: the key is the DER of
                // `RSAPublicKey ::= SEQUENCE { modulus INTEGER,
                // publicExponent INTEGER }`.
                let mut pair = [0u8; MAX_RSA_KEY];
                let mut fields = Writer::new(&mut pair);
                write_integer(&mut fields, key.modulus)?;
                write_integer(&mut fields, PUBLIC_EXPONENT)?;
                let mut outer = Writer::new(&mut bytes);
                outer.value(0x30, fields.written())?;
                let length = outer.len();
                Ok((bytes, length))
            }
        }
    }

    /// Signs `body`, writing the signature as the certificate carries it,
    /// and returns its length.
    ///
    /// # Errors
    ///
    /// [`X509Error::BufferTooSmall`] when the signature does not fit, and
    /// [`X509Error::BadSignature`] when the key cannot sign.
    pub fn sign(self, body: &[u8], out: &mut [u8]) -> Result<usize, X509Error> {
        match self {
            TestKey::Ed25519(secret) => {
                let signature = ed25519::sign(&secret, body);
                let mut writer = Writer::new(out);
                writer.extend(&signature)?;
                Ok(writer.len())
            }
            TestKey::EcdsaSha256(secret) => {
                let digest = Sha256::digest(body);
                sign_ecdsa(&secret, digest.as_ref(), out)
            }
            TestKey::EcdsaSha384(secret) => {
                let digest = Sha384::digest(body);
                let mut leftmost = [0u8; 32];
                for (slot, byte) in leftmost.iter_mut().zip(digest.as_ref()) {
                    *slot = *byte;
                }
                sign_ecdsa(&secret, &leftmost, out)
            }
            TestKey::EcdsaP384Sha384(secret) => {
                // SHA-384 is exactly as wide as the order, so the digest is
                // the scalar and nothing is truncated or padded.
                let digest = Sha384::digest(body);
                let (r, s) = p384::sign(&secret, &digest).map_err(|_| X509Error::BadSignature)?;
                write_signature(&r, &s, out)
            }
            TestKey::Rsa(key, scheme) => sign_rsa(key, scheme, body, out),
        }
    }
}

/// Signs with an RSA key, under whichever of the six schemes the key
/// carries.
///
/// The salt of a PSS signature is the digest of the body, which is as
/// long as the scheme wants and is a number this repository computed.
/// Nothing here is random: every certificate these tests build is the
/// same bytes every time.
fn sign_rsa(
    key: RsaTestKey,
    scheme: RsaScheme,
    body: &[u8],
    out: &mut [u8],
) -> Result<usize, X509Error> {
    let public = RsaKey::new(key.modulus, PUBLIC_EXPONENT).map_err(|_| X509Error::BadPublicKey)?;
    let hash = scheme.hash();
    let size = public.size();
    let mut signature = [0u8; MAX_SIGNATURE];
    let slot = signature.get_mut(..size).ok_or(X509Error::BufferTooSmall)?;
    if scheme.is_pss() {
        let salt = hash.digest(body);
        let salt = salt.get(..hash.output_len()).unwrap_or(&[]);
        sign_pss(&public, key.private_exponent, hash, body, salt, slot)
            .map_err(|_| X509Error::BadSignature)?;
    } else {
        sign_pkcs1(&public, key.private_exponent, hash, body, slot)
            .map_err(|_| X509Error::BadSignature)?;
    }
    let mut writer = Writer::new(out);
    writer.extend(signature.get(..size).unwrap_or(&[]))?;
    Ok(writer.len())
}

/// What a certificate is to say.
#[derive(Clone, Copy, Debug)]
pub struct Params<'a> {
    /// The serial number, which the builder writes as one byte.
    pub serial: u8,
    /// The common name of the issuer, which must equal the subject of the
    /// certificate above for the chain to link.
    pub issuer: &'a str,
    /// The common name of the subject.
    pub subject: &'a str,
    /// The first moment of validity.
    pub not_before: CivilTime,
    /// The last moment of validity.
    pub not_after: CivilTime,
    /// The names to put in the subject alternative name, if any.
    pub dns_names: &'a [&'a str],
    /// What `basicConstraints` is to say, if it is to be present.
    pub basic_constraints: Option<BasicConstraints>,
    /// What `keyUsage` is to say, as its first two bits bytes.
    pub key_usage: Option<u16>,
    /// Whether to write `extKeyUsage`, and whether it names server
    /// authentication.
    pub extended_key_usage: Option<bool>,
    /// An extension identifier this crate does not know, marked critical,
    /// for the test that such a certificate is refused.
    pub unknown_critical: bool,
    /// The same identifier, not marked critical, for the test that such a
    /// certificate is read and the extension passed over.
    pub unknown_harmless: bool,
    /// Whether to write the basic constraints twice, for the test that a
    /// repeated extension is refused.
    pub duplicate_extension: bool,
}

impl<'a> Params<'a> {
    /// A leaf certificate for one name, valid in the given window.
    #[must_use]
    pub const fn leaf(
        issuer: &'a str,
        subject: &'a str,
        dns_names: &'a [&'a str],
        not_before: CivilTime,
        not_after: CivilTime,
    ) -> Params<'a> {
        Params {
            serial: 1,
            issuer,
            subject,
            not_before,
            not_after,
            dns_names,
            basic_constraints: None,
            key_usage: Some(0x8000),
            extended_key_usage: Some(true),
            unknown_critical: false,
            unknown_harmless: false,
            duplicate_extension: false,
        }
    }

    /// An authority certificate that may sign `path_len` further ones.
    #[must_use]
    pub const fn authority(
        issuer: &'a str,
        subject: &'a str,
        path_len: Option<u32>,
        not_before: CivilTime,
        not_after: CivilTime,
    ) -> Params<'a> {
        Params {
            serial: 1,
            issuer,
            subject,
            not_before,
            not_after,
            dns_names: &[],
            basic_constraints: Some(BasicConstraints { ca: true, path_len }),
            key_usage: Some(0x0400),
            extended_key_usage: None,
            unknown_critical: false,
            unknown_harmless: false,
            duplicate_extension: false,
        }
    }
}

/// Writes the certificate `params` describes into `out`, signed by
/// `issuer_key`, and returns its length.
///
/// # Errors
///
/// [`X509Error::BufferTooSmall`] when the certificate does not fit, and
/// the errors of the keys.
pub fn build(
    params: &Params<'_>,
    subject_key: TestKey,
    issuer_key: TestKey,
    out: &mut [u8],
) -> Result<usize, X509Error> {
    let mut body = [0u8; MAX_CERTIFICATE];
    let mut tbs = Writer::new(&mut body);
    write_body(params, subject_key, issuer_key, &mut tbs)?;

    let mut signature = [0u8; MAX_SIGNATURE];
    let signature_len = issuer_key.sign(tbs.written(), &mut signature)?;
    let mut wrapped = [0u8; MAX_SIGNATURE + 1];
    let mut bits = Writer::new(&mut wrapped);
    bits.push(0x00)?;
    bits.extend(signature.get(..signature_len).unwrap_or(&[]))?;

    let mut inner = [0u8; MAX_CERTIFICATE];
    let mut fields = Writer::new(&mut inner);
    fields.extend(tbs.written())?;
    write_algorithm(&mut fields, issuer_key.algorithm())?;
    fields.value(0x03, bits.written())?;

    let mut outer = Writer::new(out);
    outer.value(0x30, fields.written())?;
    Ok(outer.len())
}

/// Writes the signed body.
fn write_body(
    params: &Params<'_>,
    subject_key: TestKey,
    issuer_key: TestKey,
    out: &mut Writer<'_>,
) -> Result<(), X509Error> {
    let mut buffer = [0u8; MAX_CERTIFICATE];
    let mut fields = Writer::new(&mut buffer);

    // version [0] EXPLICIT INTEGER 2
    fields.value(0xA0, &[0x02, 0x01, 0x02])?;
    fields.value(0x02, &[params.serial])?;
    write_algorithm(&mut fields, issuer_key.algorithm())?;
    write_name(&mut fields, params.issuer)?;
    write_validity(&mut fields, params)?;
    write_name(&mut fields, params.subject)?;
    write_public_key(&mut fields, subject_key)?;
    write_extensions(&mut fields, params)?;

    out.value(0x30, fields.written())
}

/// Writes an algorithm identifier.
fn write_algorithm(
    writer: &mut Writer<'_>,
    algorithm: SignatureAlgorithm,
) -> Result<(), X509Error> {
    let mut buffer = [0u8; 128];
    let mut fields = Writer::new(&mut buffer);
    match algorithm {
        // No parameters at all, which is what these three require.
        SignatureAlgorithm::EcdsaSha256 => fields.value(0x06, oid::ECDSA_WITH_SHA256)?,
        SignatureAlgorithm::EcdsaSha384 => fields.value(0x06, oid::ECDSA_WITH_SHA384)?,
        SignatureAlgorithm::Ed25519 => fields.value(0x06, oid::ED25519)?,
        // NULL, which RFC 4055, section 5 asks for and which the parser
        // also accepts absent.
        SignatureAlgorithm::RsaPkcs1Sha256 => {
            fields.value(0x06, oid::SHA256_WITH_RSA)?;
            fields.value(0x05, &[])?;
        }
        SignatureAlgorithm::RsaPkcs1Sha384 => {
            fields.value(0x06, oid::SHA384_WITH_RSA)?;
            fields.value(0x05, &[])?;
        }
        SignatureAlgorithm::RsaPkcs1Sha512 => {
            fields.value(0x06, oid::SHA512_WITH_RSA)?;
            fields.value(0x05, &[])?;
        }
        SignatureAlgorithm::RsaPssSha256 => write_pss_parameters(&mut fields, oid::SHA256, 32)?,
        SignatureAlgorithm::RsaPssSha384 => write_pss_parameters(&mut fields, oid::SHA384, 48)?,
        SignatureAlgorithm::RsaPssSha512 => write_pss_parameters(&mut fields, oid::SHA512, 64)?,
    }
    writer.value(0x30, fields.written())
}

/// Writes `id-RSASSA-PSS` and the `RSASSA-PSS-params` of RFC 4055,
/// section 3.1 for one of the three parameter sets: the hash, MGF1 over
/// that same hash, and the salt length. `trailerField` is its default and
/// so is left out, which the distinguished rules require.
fn write_pss_parameters(fields: &mut Writer<'_>, hash: &[u8], salt: u8) -> Result<(), X509Error> {
    fields.value(0x06, oid::RSASSA_PSS)?;

    let mut identifier = [0u8; 16];
    let mut inner = Writer::new(&mut identifier);
    inner.value(0x06, hash)?;
    let mut hash_algorithm = [0u8; 24];
    let mut sequence = Writer::new(&mut hash_algorithm);
    sequence.value(0x30, inner.written())?;

    let mut mask = [0u8; 48];
    let mut mask_fields = Writer::new(&mut mask);
    mask_fields.value(0x06, oid::MGF1)?;
    mask_fields.extend(sequence.written())?;
    let mut mask_algorithm = [0u8; 56];
    let mut mask_sequence = Writer::new(&mut mask_algorithm);
    mask_sequence.value(0x30, mask_fields.written())?;

    let mut length = [0u8; 8];
    let mut salt_length = Writer::new(&mut length);
    salt_length.value(0x02, &[salt])?;

    let mut buffer = [0u8; 96];
    let mut params = Writer::new(&mut buffer);
    params.value(0xA0, sequence.written())?;
    params.value(0xA1, mask_sequence.written())?;
    params.value(0xA2, salt_length.written())?;

    fields.value(0x30, params.written())
}

/// Writes a name of one common name.
fn write_name(writer: &mut Writer<'_>, common_name: &str) -> Result<(), X509Error> {
    let mut attribute = [0u8; PART];
    let mut fields = Writer::new(&mut attribute);
    fields.value(0x06, oid::COMMON_NAME)?;
    fields.value(0x0C, common_name.as_bytes())?;

    let mut pair = [0u8; PART];
    let mut sequence = Writer::new(&mut pair);
    sequence.value(0x30, fields.written())?;

    let mut set = [0u8; PART];
    let mut outer = Writer::new(&mut set);
    outer.value(0x31, sequence.written())?;

    writer.value(0x30, outer.written())
}

/// Writes the validity window.
fn write_validity(writer: &mut Writer<'_>, params: &Params<'_>) -> Result<(), X509Error> {
    let mut buffer = [0u8; PART];
    let mut fields = Writer::new(&mut buffer);
    write_time(&mut fields, params.not_before)?;
    write_time(&mut fields, params.not_after)?;
    writer.value(0x30, fields.written())
}

/// Writes one time, in the form its year prescribes.
fn write_time(writer: &mut Writer<'_>, time: CivilTime) -> Result<(), X509Error> {
    let mut digits = [0u8; 16];
    let mut text = Writer::new(&mut digits);
    let tag = if (1950..=2049).contains(&time.year) {
        write_two(&mut text, u8::try_from(time.year % 100).unwrap_or(0))?;
        0x17
    } else {
        write_two(&mut text, u8::try_from(time.year / 100).unwrap_or(0))?;
        write_two(&mut text, u8::try_from(time.year % 100).unwrap_or(0))?;
        0x18
    };
    write_two(&mut text, time.month)?;
    write_two(&mut text, time.day)?;
    write_two(&mut text, time.hour)?;
    write_two(&mut text, time.minute)?;
    write_two(&mut text, time.second)?;
    text.push(b'Z')?;
    writer.value(tag, text.written())
}

/// Writes one two-digit field.
fn write_two(writer: &mut Writer<'_>, value: u8) -> Result<(), X509Error> {
    writer.push(b'0'.wrapping_add(value / 10))?;
    writer.push(b'0'.wrapping_add(value % 10))
}

/// Writes the subject public key information.
fn write_public_key(writer: &mut Writer<'_>, key: TestKey) -> Result<(), X509Error> {
    let (public, length) = key.public_key()?;

    let mut algorithm = [0u8; 64];
    let mut fields = Writer::new(&mut algorithm);
    match key {
        TestKey::Ed25519(_) => fields.value(0x06, oid::ED25519)?,
        TestKey::EcdsaSha256(_) | TestKey::EcdsaSha384(_) => {
            fields.value(0x06, oid::EC_PUBLIC_KEY)?;
            fields.value(0x06, oid::PRIME256V1)?;
        }
        TestKey::EcdsaP384Sha384(_) => {
            fields.value(0x06, oid::EC_PUBLIC_KEY)?;
            fields.value(0x06, oid::SECP384R1)?;
        }
        TestKey::Rsa(_, _) => {
            // RFC 3279, section 2.3.1: the parameters MUST be NULL.
            fields.value(0x06, oid::RSA_ENCRYPTION)?;
            fields.value(0x05, &[])?;
        }
    }

    let mut bits = [0u8; MAX_RSA_KEY + 1];
    let mut string = Writer::new(&mut bits);
    string.push(0x00)?;
    string.extend(public.get(..length).unwrap_or(&[]))?;

    let mut buffer = [0u8; MAX_SPKI];
    let mut info = Writer::new(&mut buffer);
    info.value(0x30, fields.written())?;
    info.value(0x03, string.written())?;

    writer.value(0x30, info.written())
}

/// Writes the extensions, wrapped in their explicit tag.
fn write_extensions(writer: &mut Writer<'_>, params: &Params<'_>) -> Result<(), X509Error> {
    let mut list = [0u8; MAX_CERTIFICATE];
    let mut extensions = Writer::new(&mut list);

    write_constraints(&mut extensions, params)?;
    write_key_usage(&mut extensions, params)?;
    write_purposes(&mut extensions, params)?;
    write_names(&mut extensions, params)?;
    if params.unknown_critical || params.unknown_harmless {
        // A policy identifier, which this crate does not read.
        let unknown: &[u8] = &[0x55, 0x1D, 0x20];
        write_extension(
            &mut extensions,
            unknown,
            params.unknown_critical,
            &[0x30, 0x00],
        )?;
    }

    if extensions.len() == 0 {
        return Ok(());
    }
    let mut sequence = [0u8; MAX_CERTIFICATE];
    let mut wrapper = Writer::new(&mut sequence);
    wrapper.value(0x30, extensions.written())?;
    writer.value(0xA3, wrapper.written())
}

/// Writes `basicConstraints`, twice when the parameters ask for it.
fn write_constraints(extensions: &mut Writer<'_>, params: &Params<'_>) -> Result<(), X509Error> {
    let Some(constraints) = params.basic_constraints else {
        return Ok(());
    };

    let mut buffer = [0u8; 32];
    let mut fields = Writer::new(&mut buffer);
    if constraints.ca {
        fields.value(0x01, &[0xFF])?;
    }
    if let Some(path) = constraints.path_len {
        fields.value(0x02, &[u8::try_from(path).unwrap_or(0)])?;
    }

    let mut wrapped = [0u8; 64];
    let mut sequence = Writer::new(&mut wrapped);
    sequence.value(0x30, fields.written())?;

    write_extension(extensions, oid::BASIC_CONSTRAINTS, true, sequence.written())?;
    if params.duplicate_extension {
        write_extension(extensions, oid::BASIC_CONSTRAINTS, true, sequence.written())?;
    }
    Ok(())
}

/// Writes `keyUsage`, whose bit string carries its own count of unused
/// bits.
fn write_key_usage(extensions: &mut Writer<'_>, params: &Params<'_>) -> Result<(), X509Error> {
    let Some(usage) = params.key_usage else {
        return Ok(());
    };

    let mut buffer = [0u8; 16];
    let mut string = Writer::new(&mut buffer);
    let high = u8::try_from(usage >> 8).unwrap_or(0);
    let low = u8::try_from(usage & 0xFF).unwrap_or(0);
    if low == 0 {
        string.push(u8::try_from(high.trailing_zeros()).unwrap_or(0))?;
        string.push(high)?;
    } else {
        string.push(u8::try_from(low.trailing_zeros()).unwrap_or(0))?;
        string.push(high)?;
        string.push(low)?;
    }

    let mut wrapped = [0u8; 32];
    let mut bits = Writer::new(&mut wrapped);
    bits.value(0x03, string.written())?;
    write_extension(extensions, oid::KEY_USAGE, true, bits.written())
}

/// Writes `extKeyUsage`.
fn write_purposes(extensions: &mut Writer<'_>, params: &Params<'_>) -> Result<(), X509Error> {
    let Some(server_auth) = params.extended_key_usage else {
        return Ok(());
    };
    let identifier = if server_auth {
        oid::SERVER_AUTH
    } else {
        // Client authentication, which a server certificate must not be
        // accepted for.
        &[0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x02]
    };

    let mut buffer = [0u8; 32];
    let mut purposes = Writer::new(&mut buffer);
    purposes.value(0x06, identifier)?;

    let mut wrapped = [0u8; 64];
    let mut sequence = Writer::new(&mut wrapped);
    sequence.value(0x30, purposes.written())?;
    write_extension(
        extensions,
        oid::EXTENDED_KEY_USAGE,
        false,
        sequence.written(),
    )
}

/// Writes `subjectAltName`.
fn write_names(extensions: &mut Writer<'_>, params: &Params<'_>) -> Result<(), X509Error> {
    if params.dns_names.is_empty() {
        return Ok(());
    }

    let mut buffer = [0u8; PART];
    let mut names = Writer::new(&mut buffer);
    for name in params.dns_names {
        names.value(0x82, name.as_bytes())?;
    }

    let mut wrapped = [0u8; PART];
    let mut sequence = Writer::new(&mut wrapped);
    sequence.value(0x30, names.written())?;
    write_extension(extensions, oid::SUBJECT_ALT_NAME, false, sequence.written())
}

/// Writes one extension.
fn write_extension(
    writer: &mut Writer<'_>,
    identifier: &[u8],
    critical: bool,
    value: &[u8],
) -> Result<(), X509Error> {
    let mut buffer = [0u8; MAX_CERTIFICATE];
    let mut extension = Writer::new(&mut buffer);
    extension.value(0x06, identifier)?;
    if critical {
        extension.value(0x01, &[0xFF])?;
    }
    extension.value(0x04, value)?;
    writer.value(0x30, extension.written())
}

/// Signs a digest and writes the two integers as a sequence.
fn sign_ecdsa(secret: &[u8; 32], digest: &[u8], out: &mut [u8]) -> Result<usize, X509Error> {
    let digest: &[u8; 32] = digest.try_into().map_err(|_| X509Error::BadSignature)?;
    let (r, s) = p256::sign(secret, digest).map_err(|_| X509Error::BadSignature)?;
    write_signature(&r, &s, out)
}

/// Writes the two integers of an `ECDSA-Sig-Value` as a sequence.
fn write_signature(r: &[u8], s: &[u8], out: &mut [u8]) -> Result<usize, X509Error> {
    let mut buffer = [0u8; 112];
    let mut fields = Writer::new(&mut buffer);
    write_integer(&mut fields, r)?;
    write_integer(&mut fields, s)?;

    let mut writer = Writer::new(out);
    writer.value(0x30, fields.written())?;
    Ok(writer.len())
}

/// Writes an unsigned integer in the shortest form.
fn write_integer(writer: &mut Writer<'_>, value: &[u8]) -> Result<(), X509Error> {
    let trimmed = trim_leading_zeros(value);
    let mut buffer = [0u8; MAX_SIGNATURE + 1];
    let mut bytes = Writer::new(&mut buffer);
    match trimmed.first() {
        None => bytes.push(0x00)?,
        Some(&first) if first & 0x80 != 0 => {
            bytes.push(0x00)?;
            bytes.extend(trimmed)?;
        }
        Some(_) => bytes.extend(trimmed)?,
    }
    writer.value(0x02, bytes.written())
}

/// The value without the zeros in front of it.
const fn trim_leading_zeros(value: &[u8]) -> &[u8] {
    let mut rest = value;
    while let Some((&0x00, tail)) = rest.split_first() {
        rest = tail;
    }
    rest
}

/// A cursor that appends into a buffer the caller owns.
struct Writer<'a> {
    /// The buffer.
    buffer: &'a mut [u8],
    /// How much of it is used.
    used: usize,
}

impl<'a> Writer<'a> {
    /// A writer over `buffer`.
    const fn new(buffer: &'a mut [u8]) -> Writer<'a> {
        Writer { buffer, used: 0 }
    }

    /// How much has been written.
    const fn len(&self) -> usize {
        self.used
    }

    /// What has been written. `used` never exceeds the buffer, because
    /// every write checks first, so this is the one place where that
    /// invariant is turned back into a slice.
    fn written(&self) -> &[u8] {
        self.buffer.get(..self.used).unwrap_or(&[])
    }

    /// Appends one byte.
    fn push(&mut self, byte: u8) -> Result<(), X509Error> {
        let slot = self
            .buffer
            .get_mut(self.used)
            .ok_or(X509Error::BufferTooSmall)?;
        *slot = byte;
        self.used = self.used.wrapping_add(1);
        Ok(())
    }

    /// Appends bytes.
    fn extend(&mut self, bytes: &[u8]) -> Result<(), X509Error> {
        for byte in bytes {
            self.push(*byte)?;
        }
        Ok(())
    }

    /// Appends a value: its tag, its length in the shortest form, and its
    /// content.
    fn value(&mut self, tag: u8, content: &[u8]) -> Result<(), X509Error> {
        self.push(tag)?;
        let length = content.len();
        if length < 0x80 {
            self.push(u8::try_from(length).unwrap_or(0))?;
        } else if length <= 0xFF {
            self.push(0x81)?;
            self.push(u8::try_from(length).unwrap_or(0))?;
        } else if length <= 0xFFFF {
            self.push(0x82)?;
            self.push(u8::try_from(length >> 8).unwrap_or(0))?;
            self.push(u8::try_from(length & 0xFF).unwrap_or(0))?;
        } else {
            return Err(X509Error::BufferTooSmall);
        }
        self.extend(content)
    }
}
