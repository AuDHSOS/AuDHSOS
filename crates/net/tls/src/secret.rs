// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A secret of the length its suite decides.
//!
//! Every secret of the schedule is one hash long, and the hash is thirty-two
//! or forty-eight bytes. Carrying the longer array with a length is simpler
//! than making every step generic over the hash, and it keeps the secrets
//! in one type that clears itself.

use crypto_ct::{Choice, ct_eq, wipe};

use crate::suite::MAX_HASH;

/// A secret, at most [`MAX_HASH`] bytes.
///
/// It has no `Debug` that shows content and no equality but the
/// constant-time one.
#[derive(Clone)]
pub struct Secret {
    /// The bytes, of which the first `len` are the secret.
    bytes: [u8; MAX_HASH],
    /// How many bytes are the secret.
    len: usize,
}

impl Secret {
    /// A secret of `len` zero bytes, to be filled.
    #[must_use]
    pub const fn zero(len: usize) -> Secret {
        Secret {
            bytes: [0u8; MAX_HASH],
            len,
        }
    }

    /// The secret in `bytes`.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Secret {
        let mut secret = Secret::zero(bytes.len().min(MAX_HASH));
        for (slot, byte) in secret.bytes.iter_mut().zip(bytes) {
            *slot = *byte;
        }
        secret
    }

    /// The bytes of the secret.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }

    /// The bytes of the secret, to be written.
    #[must_use]
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        let len = self.len;
        self.bytes.get_mut(..len).unwrap_or(&mut [])
    }

    /// How long the secret is.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the secret has no bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether two secrets are equal, in time that does not depend on
    /// where they differ.
    #[must_use]
    pub fn ct_eq(&self, other: &Secret) -> Choice {
        ct_eq(self.as_bytes(), other.as_bytes())
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.bytes);
    }
}

impl core::fmt::Debug for Secret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Secret<{}>", self.len)
    }
}
