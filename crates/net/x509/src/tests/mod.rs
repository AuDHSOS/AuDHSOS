// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.
//!
//! Every certificate here is built by [`crate::builder`] and signed with
//! the deterministic signing of `crypto-ec`. Nothing is vendored: the
//! private key of every test certificate is a constant in this directory.

mod certificate;
mod name;
mod parts;
mod path;
mod rsa;

use audhsos_time::CivilTime;

use crate::builder::{MAX_CERTIFICATE, Params, TestKey, build};
use crate::error::X509Error;

/// The secret of the authority, and of everything that signs.
pub(crate) const AUTHORITY_SECRET: [u8; 32] = [0x11; 32];
/// The secret of the leaf.
pub(crate) const LEAF_SECRET: [u8; 32] = [0x22; 32];

/// A moment before every certificate these tests build.
pub(crate) fn early() -> CivilTime {
    CivilTime {
        year: 2020,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    }
}

/// A moment after every certificate these tests build.
pub(crate) fn late() -> CivilTime {
    CivilTime {
        year: 2030,
        month: 12,
        day: 31,
        hour: 23,
        minute: 59,
        second: 59,
    }
}

/// A built certificate and the bytes it lives in.
pub(crate) struct Built {
    /// The encoding.
    pub(crate) bytes: [u8; MAX_CERTIFICATE],
    /// How much of it is the certificate.
    pub(crate) length: usize,
}

impl Built {
    /// The encoded certificate.
    pub(crate) fn as_slice(&self) -> &[u8] {
        self.bytes.get(..self.length).unwrap_or(&[])
    }
}

/// Builds a certificate, or says why it could not be built.
pub(crate) fn build_certificate(
    params: &Params<'_>,
    subject: TestKey,
    issuer: TestKey,
) -> Result<Built, X509Error> {
    let mut bytes = [0u8; MAX_CERTIFICATE];
    let length = build(params, subject, issuer, &mut bytes)?;
    Ok(Built { bytes, length })
}
