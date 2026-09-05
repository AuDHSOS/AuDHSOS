// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The three cipher suites, and what each of them decides.
//!
//! A suite names a hash and an authenticated cipher, and those two decide
//! every length in the key schedule and in the record layer. Nothing else
//! in this crate reads the code point.

/// The longest hash any suite uses.
pub const MAX_HASH: usize = 48;
/// The longest key any suite uses.
pub const MAX_KEY: usize = 32;
/// The nonce length, which is twelve for all three.
pub const IV_LEN: usize = 12;

/// A cipher suite of TLS 1.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CipherSuite {
    /// `TLS_AES_128_GCM_SHA256`, which every implementation must have.
    Aes128GcmSha256,
    /// `TLS_AES_256_GCM_SHA384`.
    Aes256GcmSha384,
    /// `TLS_CHACHA20_POLY1305_SHA256`.
    ChaCha20Poly1305Sha256,
}

impl CipherSuite {
    /// The suite a code point names, or nothing when it names one this
    /// client does not offer.
    #[must_use]
    pub const fn from_code(code: u16) -> Option<CipherSuite> {
        match code {
            0x1301 => Some(CipherSuite::Aes128GcmSha256),
            0x1302 => Some(CipherSuite::Aes256GcmSha384),
            0x1303 => Some(CipherSuite::ChaCha20Poly1305Sha256),
            _ => None,
        }
    }

    /// The code point of the suite.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            CipherSuite::Aes128GcmSha256 => 0x1301,
            CipherSuite::Aes256GcmSha384 => 0x1302,
            CipherSuite::ChaCha20Poly1305Sha256 => 0x1303,
        }
    }

    /// Bytes of hash, which is also the length of every secret.
    #[must_use]
    pub const fn hash_len(self) -> usize {
        match self {
            CipherSuite::Aes128GcmSha256 | CipherSuite::ChaCha20Poly1305Sha256 => 32,
            CipherSuite::Aes256GcmSha384 => 48,
        }
    }

    /// Bytes of key.
    #[must_use]
    pub const fn key_len(self) -> usize {
        match self {
            CipherSuite::Aes128GcmSha256 => 16,
            CipherSuite::Aes256GcmSha384 | CipherSuite::ChaCha20Poly1305Sha256 => 32,
        }
    }
}
