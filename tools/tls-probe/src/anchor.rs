// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The anchor the run trusts.
//!
//! `audhsos_x509::TrustAnchor` is a subject name and a key, not a
//! certificate: whatever a root says about itself is said by the party the
//! system decided to trust. A certificate is nevertheless the shape an
//! anchor arrives in, so this module reads one and keeps the two fields.
//!
//! Which certificate to pin is a decision, not a detail; the README says
//! which one this ships with and why.
//!
//! This module only decodes. `TrustAnchor::from_certificate` is what turns
//! the bytes into an anchor, and it reads the certificate no further than
//! the key: a root's own signature is never verified, so a root that
//! reaches this system only as a cross-signed certificate — signed by an
//! older authority with an algorithm this system has no arithmetic for —
//! is an anchor like any other.

use audhsos_encoding::pem;

use crate::error::AnchorError;

/// The label RFC 7468 gives a certificate.
const LABEL: &str = "CERTIFICATE";

/// The DER of an anchor, however it was written down.
///
/// # Errors
///
/// [`AnchorError::Encoding`] for a PEM text that does not decode, and
/// [`AnchorError::WrongLabel`] for a block that is not a certificate.
pub fn der(input: &[u8]) -> Result<Vec<u8>, AnchorError> {
    let der = if input.starts_with(b"-----BEGIN") {
        // Base64 never grows, so the input length is always enough room.
        let mut out = vec![0u8; input.len()];
        let block = pem::decode(input, &mut out).map_err(AnchorError::Encoding)?;
        if block.label != LABEL {
            return Err(AnchorError::WrongLabel);
        }
        block.bytes.to_vec()
    } else {
        input.to_vec()
    };
    Ok(der)
}
