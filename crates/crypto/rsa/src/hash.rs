// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Which hash a signature was made with.
//!
//! The three are the ones RFC 8446 pairs with RSA, in both the PKCS #1
//! and the PSS schemes. A caller names one; the crate does the hashing,
//! because both encodings hash the message more than once and a caller
//! that passed a digest could not do the second one.

use crypto_hash::{Sha256, Sha384, Sha512};

use crate::window::head;

/// Bytes of the widest digest this crate produces, which is SHA-512's.
pub const MAX_DIGEST: usize = 64;

/// One of the three hash functions RSA is used with here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HashId {
    /// SHA-256, thirty-two bytes of output.
    Sha256,
    /// SHA-384, forty-eight bytes of output.
    Sha384,
    /// SHA-512, sixty-four bytes of output.
    Sha512,
}

impl HashId {
    /// Bytes of digest.
    #[must_use]
    pub const fn output_len(self) -> usize {
        match self {
            HashId::Sha256 => 32,
            HashId::Sha384 => 48,
            HashId::Sha512 => 64,
        }
    }

    /// The digest of one message, in a buffer of the widest output, of
    /// which the first [`HashId::output_len`] bytes are the digest.
    #[must_use]
    pub fn digest(self, message: &[u8]) -> [u8; MAX_DIGEST] {
        let mut out = [0u8; MAX_DIGEST];
        match self {
            HashId::Sha256 => copy(&mut out, Sha256::digest(message).as_ref()),
            HashId::Sha384 => copy(&mut out, Sha384::digest(message).as_ref()),
            HashId::Sha512 => copy(&mut out, Sha512::digest(message).as_ref()),
        }
        out
    }
}

/// The bytes of `digest` at the front of `out`.
fn copy(out: &mut [u8; MAX_DIGEST], digest: &[u8]) {
    for (slot, byte) in out.iter_mut().zip(head(digest, MAX_DIGEST)) {
        *slot = *byte;
    }
}
