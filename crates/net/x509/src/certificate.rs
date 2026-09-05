// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a certificate holds.
//!
//! Invariants: every field is a slice of the bytes that were parsed, so
//! the body that was signed is the body that is hashed and not a
//! re-encoding of it; an extension this crate does not understand rejects
//! the certificate when it is marked critical; and a field with a default
//! value must be absent, as the distinguished encoding rules require.

use audhsos_der::{DerError, Reader, Tag};
use audhsos_time::CivilTime;

use crate::algorithm::{DNS_NAME_TAG, SignatureAlgorithm, SubjectPublicKey, expect_same};
use crate::error::X509Error;
use crate::oid;

/// The window a certificate is valid in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Validity {
    /// The first moment the certificate is valid.
    pub not_before: CivilTime,
    /// The last moment the certificate is valid.
    pub not_after: CivilTime,
}

/// What `basicConstraints` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BasicConstraints {
    /// Whether the subject may sign certificates.
    pub ca: bool,
    /// How many certificates may follow this one before the end entity.
    pub path_len: Option<u32>,
}

/// What `keyUsage` says, as the bits of its string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyUsage {
    /// The bits, most significant first, as the encoding orders them.
    bits: u16,
}

impl KeyUsage {
    /// Whether bit `position` is set, counting from the first bit of the
    /// string.
    #[must_use]
    pub fn has(self, position: u8) -> bool {
        let mask = 0x8000u16.wrapping_shr(u32::from(position));
        self.bits & mask != 0
    }

    /// Whether the key may sign certificates, which is bit five.
    #[must_use]
    pub fn key_cert_sign(self) -> bool {
        self.has(5)
    }

    /// Whether the key may sign, which is bit zero.
    #[must_use]
    pub fn digital_signature(self) -> bool {
        self.has(0)
    }
}

/// A parsed certificate. Every field borrows from the input.
#[derive(Clone, Copy, Debug)]
pub struct Certificate<'a> {
    /// The exact bytes that were signed, tag and length included.
    pub tbs: &'a [u8],
    /// The serial number, without a padding zero.
    pub serial: &'a [u8],
    /// The issuer name, as the bytes it is encoded as.
    pub issuer: &'a [u8],
    /// The subject name, as the bytes it is encoded as.
    pub subject: &'a [u8],
    /// The validity window.
    pub validity: Validity,
    /// The public key.
    pub spki: SubjectPublicKey<'a>,
    /// The subject public key information, as the bytes it is encoded as,
    /// which is what a key identifier is computed over.
    pub spki_bytes: &'a [u8],
    /// The algorithm of the signature on this certificate.
    pub algorithm: SignatureAlgorithm,
    /// The signature.
    pub signature: &'a [u8],
    /// What `basicConstraints` says, when it is present.
    pub basic_constraints: Option<BasicConstraints>,
    /// What `keyUsage` says, when it is present.
    pub key_usage: Option<KeyUsage>,
    /// Whether an `extKeyUsage` extension is present, and whether it names
    /// server authentication.
    pub extended_key_usage: Option<bool>,
    /// The subject alternative name, as the content of its extension.
    pub subject_alt_name: Option<&'a [u8]>,
    /// The subject key identifier, when it is present.
    pub subject_key_identifier: Option<&'a [u8]>,
}

impl<'a> Certificate<'a> {
    /// The certificate the bytes encode.
    ///
    /// # Errors
    ///
    /// [`X509Error`] names which rule was broken; the encoding errors of
    /// the reader arrive wrapped.
    pub fn parse(bytes: &'a [u8]) -> Result<Certificate<'a>, X509Error> {
        let mut outer = Reader::new(bytes);
        let mut certificate = outer.read_sequence()?;
        outer.finish()?;

        let (mut body, tbs) = take_sequence(&mut certificate)?;
        let parsed = Certificate::parse_body(&mut body, tbs)?;
        body.finish()?;

        expect_same(&mut certificate, parsed.algorithm)?;
        let signature = certificate.read_bit_string()?.whole_bytes()?;
        certificate.finish()?;

        Ok(Certificate {
            signature,
            ..parsed
        })
    }

    /// The fields inside the signed body. The signature itself is filled in
    /// by the caller, which is the only field that lives outside.
    fn parse_body(body: &mut Reader<'a>, tbs: &'a [u8]) -> Result<Certificate<'a>, X509Error> {
        let mut version = body
            .read_context(0)
            .map_err(|_| X509Error::NotVersionThree)?;
        if version.read_integer()? != [0x02] {
            return Err(X509Error::NotVersionThree);
        }
        version.finish()?;

        let serial = body.read_integer()?;
        let algorithm = SignatureAlgorithm::parse(body)?;
        let Identity {
            issuer,
            validity,
            subject,
            spki,
            spki_bytes,
        } = read_identity(body)?;

        // The two unique identifiers of version two, which nothing issues
        // any more but the encoding still allows.
        let _ = body.read_optional(Tag::context(1, false))?;
        let _ = body.read_optional(Tag::context(2, false))?;

        let mut certificate = Certificate {
            tbs,
            serial,
            issuer,
            subject,
            validity,
            spki,
            spki_bytes,
            algorithm,
            signature: &[],
            basic_constraints: None,
            key_usage: None,
            extended_key_usage: None,
            subject_alt_name: None,
            subject_key_identifier: None,
        };

        if body.peek_is(Tag::context(3, true)) {
            let mut wrapper = body.read_context(3)?;
            let mut extensions = wrapper.read_sequence()?;
            certificate.read_extensions(&mut extensions)?;
            extensions.finish()?;
            wrapper.finish()?;
        }
        Ok(certificate)
    }

    /// Reads the extensions, rejecting a critical one this crate does not
    /// understand and a known one that appears twice.
    fn read_extensions(&mut self, extensions: &mut Reader<'a>) -> Result<(), X509Error> {
        while !extensions.is_empty() {
            let mut extension = extensions.read_sequence()?;
            let identifier = extension.read_object_identifier()?;

            let critical = if extension.peek_is(Tag::BOOLEAN) {
                if extension.read_boolean()? {
                    true
                } else {
                    // The default is false, so encoding it is forbidden.
                    return Err(X509Error::DefaultEncoded);
                }
            } else {
                false
            };

            let value = extension.read_octet_string()?;
            extension.finish()?;

            match identifier.as_bytes() {
                oid::BASIC_CONSTRAINTS => {
                    self.set_basic_constraints(parse_basic_constraints(value)?)?;
                }
                oid::KEY_USAGE => self.set_key_usage(parse_key_usage(value)?)?,
                oid::EXTENDED_KEY_USAGE => {
                    self.set_extended_key_usage(parse_extended_key_usage(value)?)?;
                }
                oid::SUBJECT_ALT_NAME => self.set_subject_alt_name(value)?,
                oid::SUBJECT_KEY_IDENTIFIER => {
                    let mut reader = Reader::new(value);
                    let identifier = reader.read_octet_string()?;
                    reader.finish()?;
                    self.set_subject_key_identifier(identifier)?;
                }
                oid::AUTHORITY_KEY_IDENTIFIER => {
                    // Read for its shape; chaining goes by name, so the
                    // value itself is not kept.
                    let mut reader = Reader::new(value);
                    reader.read_sequence()?;
                    reader.finish()?;
                }
                _ if critical => return Err(X509Error::UnknownCriticalExtension),
                _ => {}
            }
        }
        Ok(())
    }

    /// Every entry of the subject alternative name, with its tag.
    #[must_use]
    pub fn general_names(&self) -> GeneralNames<'a> {
        GeneralNames {
            reader: self.subject_alt_name.map(Reader::new),
        }
    }

    /// The names the subject alternative name carries, `dNSName` entries
    /// only.
    #[must_use]
    pub fn dns_names(&self) -> DnsNames<'a> {
        DnsNames {
            names: self.general_names(),
        }
    }

    /// Whether `signature` on this certificate is genuine under `issuer`.
    ///
    /// # Errors
    ///
    /// The errors of [`SubjectPublicKey::verify`].
    pub fn verify_signature(&self, issuer: &Certificate<'_>) -> Result<(), X509Error> {
        issuer.spki.verify(self.algorithm, self.tbs, self.signature)
    }

    /// Records the basic constraints, refusing a second appearance.
    const fn set_basic_constraints(&mut self, value: BasicConstraints) -> Result<(), X509Error> {
        if self.basic_constraints.is_some() {
            return Err(X509Error::DuplicateExtension);
        }
        self.basic_constraints = Some(value);
        Ok(())
    }

    /// Records the key usage, refusing a second appearance.
    const fn set_key_usage(&mut self, value: KeyUsage) -> Result<(), X509Error> {
        if self.key_usage.is_some() {
            return Err(X509Error::DuplicateExtension);
        }
        self.key_usage = Some(value);
        Ok(())
    }

    /// Records the extended key usage, refusing a second appearance.
    const fn set_extended_key_usage(&mut self, value: bool) -> Result<(), X509Error> {
        if self.extended_key_usage.is_some() {
            return Err(X509Error::DuplicateExtension);
        }
        self.extended_key_usage = Some(value);
        Ok(())
    }

    /// Records the subject alternative name, refusing a second appearance.
    fn set_subject_alt_name(&mut self, value: &'a [u8]) -> Result<(), X509Error> {
        if self.subject_alt_name.is_some() {
            return Err(X509Error::DuplicateExtension);
        }
        let mut reader = Reader::new(value);
        let names = reader.read_sequence()?;
        reader.finish()?;
        self.subject_alt_name = Some(names.rest());
        Ok(())
    }

    /// Records the subject key identifier, refusing a second appearance.
    const fn set_subject_key_identifier(&mut self, value: &'a [u8]) -> Result<(), X509Error> {
        if self.subject_key_identifier.is_some() {
            return Err(X509Error::DuplicateExtension);
        }
        self.subject_key_identifier = Some(value);
        Ok(())
    }
}

/// Every entry of a subject alternative name, with the tag that says what
/// kind of name it is.
pub struct GeneralNames<'a> {
    /// The remaining entries, or nothing when the extension is absent.
    reader: Option<Reader<'a>>,
}

impl GeneralNames<'_> {
    /// The entries inside an encoded `GeneralNames`, for a caller that has
    /// the content without a certificate around it.
    #[must_use]
    pub const fn over(bytes: &[u8]) -> GeneralNames<'_> {
        GeneralNames {
            reader: Some(Reader::new(bytes)),
        }
    }
}

impl<'a> Iterator for GeneralNames<'a> {
    type Item = Result<(Tag, &'a [u8]), X509Error>;

    fn next(&mut self) -> Option<Result<(Tag, &'a [u8]), X509Error>> {
        let reader = self.reader.as_mut()?;
        if reader.is_empty() {
            return None;
        }
        match reader.read_any() {
            Ok(entry) => Some(Ok(entry)),
            Err(error) => {
                self.reader = None;
                Some(Err(X509Error::Encoding(error)))
            }
        }
    }
}

/// The `dNSName` entries of a subject alternative name.
pub struct DnsNames<'a> {
    /// Every entry, of which this iterator yields some.
    names: GeneralNames<'a>,
}

impl DnsNames<'_> {
    /// The names inside an encoded `GeneralNames`, for a test that has the
    /// content without a certificate around it.
    #[cfg(test)]
    pub(crate) const fn from_names(bytes: &[u8]) -> DnsNames<'_> {
        DnsNames {
            names: GeneralNames::over(bytes),
        }
    }
}

impl<'a> Iterator for DnsNames<'a> {
    type Item = Result<&'a [u8], X509Error>;

    fn next(&mut self) -> Option<Result<&'a [u8], X509Error>> {
        loop {
            match self.names.next()? {
                Ok((tag, content)) if tag == DNS_NAME_TAG => return Some(Ok(content)),
                Ok(_) => {}
                Err(error) => return Some(Err(error)),
            }
        }
    }
}

/// The bytes a reader consumed since it stood at `before`.
fn consumed<'a>(before: &'a [u8], reader: &Reader<'a>) -> Result<&'a [u8], X509Error> {
    let length = before.len().saturating_sub(reader.rest().len());
    let (taken, _) = before
        .split_at_checked(length)
        .ok_or(X509Error::Encoding(DerError::Truncated))?;
    Ok(taken)
}

/// Who a certificate was issued by, when it is valid, who it is for, and
/// the key it carries.
///
/// These are the fields between the algorithm named inside the signed body
/// and the identifiers that may follow the key. They are a piece of their
/// own because an anchor needs two of them and nothing else:
/// [`crate::path::TrustAnchor::from_certificate`] reads a body this far
/// and stops, without ever asking what the certificate is signed with.
pub(crate) struct Identity<'a> {
    /// The issuer name, as the bytes it is encoded as.
    pub(crate) issuer: &'a [u8],
    /// The window.
    pub(crate) validity: Validity,
    /// The subject name, as the bytes it is encoded as.
    pub(crate) subject: &'a [u8],
    /// The key.
    pub(crate) spki: SubjectPublicKey<'a>,
    /// The subject public key information, as the bytes it is encoded as.
    pub(crate) spki_bytes: &'a [u8],
}

/// Reads those four fields, leaving the reader after the key.
pub(crate) fn read_identity<'a>(body: &mut Reader<'a>) -> Result<Identity<'a>, X509Error> {
    let (_, issuer) = take_sequence(body)?;

    let mut window = body.read_sequence()?;
    let not_before = window.read_time()?;
    let not_after = window.read_time()?;
    window.finish()?;

    let (_, subject) = take_sequence(body)?;

    let before_key = body.rest();
    let spki = SubjectPublicKey::parse(body)?;
    let spki_bytes = consumed(before_key, body)?;

    Ok(Identity {
        issuer,
        validity: Validity {
            not_before,
            not_after,
        },
        subject,
        spki,
        spki_bytes,
    })
}

/// A sequence and the bytes it occupies, tag and length included.
fn take_sequence<'a>(reader: &mut Reader<'a>) -> Result<(Reader<'a>, &'a [u8]), X509Error> {
    let before = reader.rest();
    let inner = reader.read_sequence()?;
    Ok((inner, consumed(before, reader)?))
}

/// The content of a `basicConstraints` extension.
pub(crate) fn parse_basic_constraints(value: &[u8]) -> Result<BasicConstraints, X509Error> {
    let mut outer = Reader::new(value);
    let mut fields = outer.read_sequence()?;
    outer.finish()?;

    let ca = if fields.peek_is(Tag::BOOLEAN) {
        if fields.read_boolean()? {
            true
        } else {
            return Err(X509Error::DefaultEncoded);
        }
    } else {
        false
    };

    let path_len = if fields.peek_is(Tag::INTEGER) {
        let bytes = fields.read_integer()?;
        let mut value = 0u32;
        for byte in bytes {
            value = value
                .checked_mul(256)
                .and_then(|shifted| shifted.checked_add(u32::from(*byte)))
                .ok_or(X509Error::BadExtension)?;
        }
        Some(value)
    } else {
        None
    };
    fields.finish()?;

    if path_len.is_some() && !ca {
        // A path length on something that cannot sign says nothing.
        return Err(X509Error::BadExtension);
    }
    Ok(BasicConstraints { ca, path_len })
}

/// The content of a `keyUsage` extension.
pub(crate) fn parse_key_usage(value: &[u8]) -> Result<KeyUsage, X509Error> {
    let mut outer = Reader::new(value);
    let string = outer.read_bit_string()?;
    outer.finish()?;

    let mut bits = 0u16;
    for (index, byte) in string.bytes.iter().enumerate().take(2) {
        let shift = if index == 0 { 8 } else { 0 };
        bits |= u16::from(*byte).wrapping_shl(shift);
    }
    if string.bytes.is_empty() {
        return Err(X509Error::BadExtension);
    }
    Ok(KeyUsage { bits })
}

/// Whether an `extKeyUsage` extension names server authentication.
pub(crate) fn parse_extended_key_usage(value: &[u8]) -> Result<bool, X509Error> {
    let mut outer = Reader::new(value);
    let mut purposes = outer.read_sequence()?;
    outer.finish()?;

    let mut found = false;
    let mut any = false;
    while !purposes.is_empty() {
        let purpose = purposes.read_object_identifier()?;
        any = true;
        if purpose.as_bytes() == oid::SERVER_AUTH {
            found = true;
        }
    }
    if any {
        Ok(found)
    } else {
        Err(X509Error::BadExtension)
    }
}
