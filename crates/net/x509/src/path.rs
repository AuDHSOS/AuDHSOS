// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Whether a chain of certificates reaches something the system trusts.
//!
//! The subset of RFC 5280 this implements is the one a TLS client needs:
//! names chained by their encodings, one signature verified per link, the
//! validity window of every certificate, the authority bit and the path
//! length on every intermediate, the certificate-signing usage where a
//! usage is stated, and server authentication on the leaf where a purpose
//! is stated.
//!
//! What is absent is absent by decision D-44: no revocation, no name
//! constraints, no policy processing. A revoked certificate that is
//! otherwise well formed passes here, and section 11.9 of document 11
//! says so rather than leaving it to be discovered.
//!
//! Invariant: a chain is walked from the leaf upwards, each step consumes
//! one intermediate, and no intermediate is used twice, so the walk cannot
//! loop.

use audhsos_der::{Reader, Timestamp};

use crate::algorithm::SubjectPublicKey;
use crate::certificate::Certificate;
use crate::error::X509Error;
use crate::name::{ServerName, matches};

/// The longest chain this crate follows, the leaf included.
pub const MAX_CHAIN: usize = 8;

/// Something the system trusts: a name and the key that goes with it.
///
/// An anchor is not a certificate. Whatever a root certificate says about
/// itself is said by the party the system decided to trust, so only the
/// two fields that are used are carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrustAnchor<'a> {
    /// The subject name, as the bytes it is encoded as.
    pub subject: &'a [u8],
    /// The subject public key information, as the bytes it is encoded as.
    pub spki: &'a [u8],
}

/// The anchors a system trusts.
#[derive(Clone, Copy, Debug)]
pub struct TrustAnchors<'a> {
    /// The anchors.
    anchors: &'a [TrustAnchor<'a>],
}

impl<'a> TrustAnchors<'a> {
    /// The anchors in `list`.
    #[must_use]
    pub const fn new(list: &'a [TrustAnchor<'a>]) -> TrustAnchors<'a> {
        TrustAnchors { anchors: list }
    }

    /// The anchor whose subject is `issuer`, if the system has one.
    #[must_use]
    fn find(&self, issuer: &[u8]) -> Option<&TrustAnchor<'a>> {
        self.anchors.iter().find(|anchor| anchor.subject == issuer)
    }
}

/// Whether `end_entity` is trustworthy for `name` at `now`, following
/// `intermediates` up to one of `anchors`.
///
/// The intermediates may be in any order and may contain certificates that
/// belong to no chain; the server sends what it has, and RFC 8446 does not
/// promise an order.
///
/// # Errors
///
/// One of the [`X509Error`] variants that name a rule of the path: the
/// window, the authority bit, the path length, the usages, the name, or
/// the absence of an anchor.
pub fn verify_chain(
    end_entity: &Certificate<'_>,
    intermediates: &[Certificate<'_>],
    anchors: &TrustAnchors<'_>,
    name: ServerName<'_>,
    now: Timestamp,
) -> Result<(), X509Error> {
    if intermediates.len() > MAX_CHAIN {
        // A server that sends more than this has not sent a chain.
        return Err(X509Error::ChainTooLong);
    }
    check_window(end_entity, now)?;
    if end_entity.extended_key_usage == Some(false) {
        return Err(X509Error::NotForServerAuthentication);
    }
    if !matches(end_entity, name)? {
        return Err(X509Error::NameMismatch);
    }

    let mut current = end_entity;
    let mut used = [false; MAX_CHAIN];
    // How many intermediates already stand between the leaf and the next
    // issuer, which is what a path length constrains.
    let mut below = 0usize;

    for _ in 0..MAX_CHAIN {
        if let Some(anchor) = anchors.find(current.issuer) {
            let mut reader = Reader::new(anchor.spki);
            let key = SubjectPublicKey::parse(&mut reader)?;
            reader.finish()?;
            return key.verify(current.algorithm, current.tbs, current.signature);
        }

        let (index, issuer) = next_issuer(current, intermediates, used)?;
        if let Some(slot) = used.get_mut(index) {
            *slot = true;
        }
        current.verify_signature(issuer)?;
        check_authority(issuer, now, below)?;
        current = issuer;
        below = below.wrapping_add(1);
    }
    Err(X509Error::ChainTooLong)
}

/// The next certificate up: one whose subject is the issuer of `current`,
/// that has not been used, and whose signature on `current` is genuine.
fn next_issuer<'a, 'b>(
    current: &Certificate<'_>,
    intermediates: &'b [Certificate<'a>],
    used: [bool; MAX_CHAIN],
) -> Result<(usize, &'b Certificate<'a>), X509Error> {
    for (index, candidate) in intermediates.iter().enumerate() {
        if used.get(index).copied().unwrap_or(true) {
            continue;
        }
        if candidate.subject == current.issuer && current.verify_signature(candidate).is_ok() {
            return Ok((index, candidate));
        }
    }
    Err(X509Error::NoTrustAnchor)
}

/// Whether `certificate` is inside its window at `now`.
fn check_window(certificate: &Certificate<'_>, now: Timestamp) -> Result<(), X509Error> {
    if now < certificate.validity.not_before {
        return Err(X509Error::NotYetValid);
    }
    if now > certificate.validity.not_after {
        return Err(X509Error::Expired);
    }
    Ok(())
}

/// Whether `issuer` may have signed a certificate that has `below`
/// intermediates under it.
fn check_authority(
    issuer: &Certificate<'_>,
    now: Timestamp,
    below: usize,
) -> Result<(), X509Error> {
    check_window(issuer, now)?;

    let constraints = issuer.basic_constraints.ok_or(X509Error::NotAnAuthority)?;
    if !constraints.ca {
        return Err(X509Error::NotAnAuthority);
    }
    if let Some(limit) = constraints.path_len {
        let allowed = usize::try_from(limit).unwrap_or(usize::MAX);
        if below > allowed {
            return Err(X509Error::PathLengthExceeded);
        }
    }
    if let Some(usage) = issuer.key_usage
        && !usage.key_cert_sign()
    {
        return Err(X509Error::NotForCertificateSigning);
    }
    Ok(())
}
