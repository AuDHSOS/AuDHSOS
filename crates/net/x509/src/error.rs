// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a certificate was rejected.

use core::fmt;

use audhsos_der::DerError;

/// Why a certificate was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum X509Error {
    /// The encoding does not follow the rules; the reader says which.
    Encoding(DerError),
    /// The certificate is not version three. Every certificate a
    /// TLS server presents is.
    NotVersionThree,
    /// The algorithm named inside the signed body differs from the one
    /// named beside the signature.
    AlgorithmMismatch,
    /// The signature algorithm or the key algorithm is not one of the
    /// three this system verifies.
    UnsupportedAlgorithm,
    /// A public key does not have the shape its algorithm prescribes.
    BadPublicKey,
    /// A signature does not have the shape its algorithm prescribes.
    BadSignature,
    /// The signature does not belong to this body and this key.
    SignatureFailed,
    /// An extension appears twice.
    DuplicateExtension,
    /// An extension this crate does not understand is marked critical.
    UnknownCriticalExtension,
    /// An extension does not have the shape its identifier prescribes.
    BadExtension,
    /// A field with a default value is present with that value, which the
    /// distinguished encoding rules forbid.
    DefaultEncoded,
    /// The name is not one this crate can match against, or is malformed.
    BadName,
    /// The buffer a certificate was to be written into is too small.
    BufferTooSmall,
}

impl From<DerError> for X509Error {
    fn from(error: DerError) -> X509Error {
        X509Error::Encoding(error)
    }
}

impl fmt::Display for X509Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            X509Error::Encoding(error) => write!(f, "the encoding is wrong: {error}"),
            X509Error::NotVersionThree => f.write_str("the certificate is not version three"),
            X509Error::AlgorithmMismatch => {
                f.write_str("the signature algorithm differs inside and outside the body")
            }
            X509Error::UnsupportedAlgorithm => f.write_str("the algorithm is not supported"),
            X509Error::BadPublicKey => f.write_str("the public key has the wrong shape"),
            X509Error::BadSignature => f.write_str("the signature has the wrong shape"),
            X509Error::SignatureFailed => f.write_str("the signature does not verify"),
            X509Error::DuplicateExtension => f.write_str("an extension appears twice"),
            X509Error::UnknownCriticalExtension => {
                f.write_str("an unknown extension is marked critical")
            }
            X509Error::BadExtension => f.write_str("an extension has the wrong shape"),
            X509Error::DefaultEncoded => f.write_str("a default value is encoded"),
            X509Error::BadName => f.write_str("the name cannot be matched"),
            X509Error::BufferTooSmall => f.write_str("the buffer is too small"),
        }
    }
}
