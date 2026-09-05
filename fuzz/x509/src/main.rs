// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Certificates against arbitrary bytes: no input may panic, everything a
//! parsed certificate points at lies inside the input it was parsed from,
//! and nothing verifies against an empty trust store.
//!
//! The last of those is the one that matters. A parser that accepts a
//! damaged certificate is a nuisance; a path validation that accepts one
//! without an anchor behind it is the whole failure this crate exists to
//! prevent.

#![cfg_attr(fuzzing, no_main)]

use audhsos_der::Timestamp;
use audhsos_x509::{Certificate, ServerName, TrustAnchors, matches, verify_chain};

/// A moment inside the window of any certificate a test would build.
const NOW: Timestamp = Timestamp {
    year: 2025,
    month: 6,
    day: 15,
    hour: 12,
    minute: 0,
    second: 0,
};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    check(bytes);
});

/// Everything a caller does with bytes that claim to be a certificate.
fn check(bytes: &[u8]) {
    let Ok(certificate) = Certificate::parse(bytes) else {
        return;
    };

    // What was parsed is a view of what was handed in.
    for slice in [
        certificate.tbs,
        certificate.serial,
        certificate.issuer,
        certificate.subject,
        certificate.spki_bytes,
        certificate.signature,
    ] {
        assert!(slice.len() <= bytes.len(), "a field larger than the input");
    }

    // Everything a caller reads off a certificate it has parsed.
    for name in certificate.dns_names() {
        let _ = name;
    }
    for entry in certificate.general_names() {
        let _ = entry;
    }
    let _ = matches(&certificate, ServerName::Dns("example.test"));
    let _ = matches(&certificate, ServerName::Ip(&[127, 0, 0, 1]));
    let _ = certificate.verify_signature(&certificate);

    // Nothing reaches a trust store that is empty.
    let anchors = TrustAnchors::new(&[]);
    assert!(
        verify_chain(
            &certificate,
            &[],
            &anchors,
            ServerName::Dns("example.test"),
            NOW,
        )
        .is_err(),
        "a certificate that verified against no anchor at all"
    );
    assert!(
        verify_chain(
            &certificate,
            &[certificate],
            &anchors,
            ServerName::Dns("example.test"),
            NOW,
        )
        .is_err(),
        "a certificate that verified as its own issuer"
    );
}
