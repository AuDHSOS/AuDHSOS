// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a caller decides before a connection begins.

use audhsos_der::Timestamp;
use audhsos_x509::TrustAnchors;

use crate::suite::CipherSuite;

/// The suites this client offers when a caller names none.
pub const DEFAULT_SUITES: [CipherSuite; 3] = [
    CipherSuite::Aes128GcmSha256,
    CipherSuite::Aes256GcmSha384,
    CipherSuite::ChaCha20Poly1305Sha256,
];

/// What a client needs to know before it speaks.
#[derive(Clone, Copy, Debug)]
pub struct ClientConfig<'a> {
    /// The name the connection is to, which is both sent and checked.
    pub server_name: &'a str,
    /// What the system trusts.
    pub anchors: TrustAnchors<'a>,
    /// The protocols to offer, in order of preference.
    pub alpn: &'a [&'a [u8]],
    /// The suites to offer, in order of preference.
    pub suites: &'a [CipherSuite],
    /// The moment to check certificate windows against.
    ///
    /// It is a parameter because this system has no clock yet; when
    /// `audhsos-time` arrives it becomes an instant, and the field keeps
    /// its place. Section 11.14 of document 11 carries the seam.
    pub now: Timestamp,
}

impl<'a> ClientConfig<'a> {
    /// A configuration with the three suites and no protocols.
    #[must_use]
    pub const fn new(
        server_name: &'a str,
        anchors: TrustAnchors<'a>,
        now: Timestamp,
    ) -> ClientConfig<'a> {
        ClientConfig {
            server_name,
            anchors,
            alpn: &[],
            suites: &DEFAULT_SUITES,
            now,
        }
    }
}
