// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The object identifiers this crate knows, as the bytes they encode to.
//!
//! An identifier is compared, never decoded, so the constants are the
//! encodings themselves. Each carries the dotted form in its documentation
//! so that a reader can check it against the standard that assigns it.

/// `ecdsa-with-SHA256`, 1.2.840.10045.4.3.2.
pub const ECDSA_WITH_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x02];
/// `ecdsa-with-SHA384`, 1.2.840.10045.4.3.3.
pub const ECDSA_WITH_SHA384: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x03];
/// `id-Ed25519`, 1.3.101.112, which names both the key and the signature.
pub const ED25519: &[u8] = &[0x2B, 0x65, 0x70];
/// `id-ecPublicKey`, 1.2.840.10045.2.1.
pub const EC_PUBLIC_KEY: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
/// `prime256v1`, 1.2.840.10045.3.1.7, the curve of P-256.
pub const PRIME256V1: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];
/// `secp384r1`, 1.3.132.0.34, the curve of P-384. RFC 5480, section
/// 2.1.1.1 assigns it; the arc is SECG's, not ANSI's, which is why it does
/// not sit beside `prime256v1`.
pub const SECP384R1: &[u8] = &[0x2B, 0x81, 0x04, 0x00, 0x22];
/// `id-at-commonName`, 2.5.4.3.
pub const COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
/// `id-ce-basicConstraints`, 2.5.29.19.
pub const BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1D, 0x13];
/// `id-ce-keyUsage`, 2.5.29.15.
pub const KEY_USAGE: &[u8] = &[0x55, 0x1D, 0x0F];
/// `id-ce-extKeyUsage`, 2.5.29.37.
pub const EXTENDED_KEY_USAGE: &[u8] = &[0x55, 0x1D, 0x25];
/// `id-ce-subjectAltName`, 2.5.29.17.
pub const SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1D, 0x11];
/// `id-ce-subjectKeyIdentifier`, 2.5.29.14.
pub const SUBJECT_KEY_IDENTIFIER: &[u8] = &[0x55, 0x1D, 0x0E];
/// `id-ce-authorityKeyIdentifier`, 2.5.29.35.
pub const AUTHORITY_KEY_IDENTIFIER: &[u8] = &[0x55, 0x1D, 0x23];
/// `id-kp-serverAuth`, 1.3.6.1.5.5.7.3.1.
pub const SERVER_AUTH: &[u8] = &[0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];
