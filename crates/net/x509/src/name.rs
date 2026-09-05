// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Whether the name at the end of a chain is the name that was asked for,
//! by the rules of RFC 6125.
//!
//! Only the subject alternative name is consulted. The common name is not
//! a fallback: a certificate that says one thing in its extension and
//! another in its subject is a certificate two systems read differently,
//! and the extension has been the authority for two decades.
//!
//! A wildcard is allowed as the whole of the leftmost label and nowhere
//! else. It matches exactly one label, so `*.example.test` covers
//! `a.example.test` and neither `example.test` nor `a.b.example.test`. A
//! partial label such as `w*.example.test` is not a wildcard here, and a
//! star anywhere else makes the name unusable rather than literal.

use audhsos_der::Tag;

use crate::algorithm::DNS_NAME_TAG;
use crate::certificate::Certificate;
use crate::error::X509Error;

/// The tag of an `iPAddress` inside a subject alternative name.
const IP_ADDRESS_TAG: Tag = Tag::context(7, false);

/// The name a client asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerName<'a> {
    /// A domain name.
    Dns(&'a str),
    /// An address, four bytes or sixteen.
    Ip(&'a [u8]),
}

/// Whether `certificate` carries a name that matches `reference`.
///
/// # Errors
///
/// [`X509Error::Encoding`] when the extension is malformed, which ends the
/// search rather than skipping past it.
pub fn matches(
    certificate: &Certificate<'_>,
    reference: ServerName<'_>,
) -> Result<bool, X509Error> {
    for entry in certificate.general_names() {
        let (tag, presented) = entry?;
        if matches_entry(tag, presented, reference) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether one entry of a subject alternative name matches.
#[must_use]
pub fn matches_entry(tag: Tag, presented: &[u8], reference: ServerName<'_>) -> bool {
    match (tag, reference) {
        (DNS_NAME_TAG, ServerName::Dns(wanted)) => matches_dns(presented, wanted.as_bytes()),
        (IP_ADDRESS_TAG, ServerName::Ip(wanted)) => presented == wanted,
        _ => false,
    }
}

/// Whether a presented domain name matches the one that was asked for.
fn matches_dns(presented: &[u8], reference: &[u8]) -> bool {
    let presented = without_trailing_dot(presented);
    let reference = without_trailing_dot(reference);
    if presented.is_empty() || reference.is_empty() {
        return false;
    }

    if let Some(rest) = strip_wildcard(presented) {
        // The wildcard stands for exactly one label, so the reference must
        // have a label to give up, and what is left must match the rest.
        let Some(position) = reference.iter().position(|byte| *byte == b'.') else {
            return false;
        };
        let after = reference.get(position.wrapping_add(1)..).unwrap_or(&[]);
        return !after.is_empty() && equal_ignoring_case(rest, after);
    }

    // A star that is not the whole leftmost label is not a wildcard, and
    // treating it as a literal would let a certificate for `w*.a.test`
    // match a host that never existed.
    if presented.contains(&b'*') {
        return false;
    }
    equal_ignoring_case(presented, reference)
}

/// What follows the wildcard, when the name begins with one.
fn strip_wildcard(name: &[u8]) -> Option<&[u8]> {
    let rest = name.strip_prefix(b"*.")?;
    if rest.is_empty() { None } else { Some(rest) }
}

/// The name without a trailing dot, which is the same name.
const fn without_trailing_dot(name: &[u8]) -> &[u8] {
    match name.split_last() {
        Some((&b'.', rest)) => rest,
        _ => name,
    }
}

/// Whether two names are equal, comparing letters without regard to case,
/// as domain names are compared.
fn equal_ignoring_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}
