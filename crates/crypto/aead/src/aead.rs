// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What every authenticated cipher in this crate provides.
//!
//! Invariants of every implementation: `seal` encrypts in place and returns
//! the tag; `open` verifies before it decrypts, so unauthenticated plaintext
//! never exists, and clears the buffer when verification fails; a nonce is
//! never reused by the crate, because the crate never chooses one.

use crate::error::AeadError;

/// Bytes of tag. The three cipher suites of TLS 1.3 all authenticate with
/// sixteen.
pub const TAG_LEN: usize = 16;

/// The tag `seal` produces and `open` checks.
pub type Tag = [u8; TAG_LEN];

/// An authenticated cipher with associated data.
pub trait Aead: Sized {
    /// Bytes of key.
    const KEY_LEN: usize;
    /// Bytes of nonce.
    const NONCE_LEN: usize;

    /// A cipher under `key`.
    ///
    /// # Errors
    ///
    /// [`AeadError::KeyLength`] when `key` is not [`Aead::KEY_LEN`] bytes.
    fn new(key: &[u8]) -> Result<Self, AeadError>;

    /// Encrypts `in_out` in place and returns the tag over the ciphertext
    /// and `aad`.
    ///
    /// The caller owns the nonce and must never repeat one under the same
    /// key. In TLS the record sequence number provides it.
    ///
    /// # Errors
    ///
    /// [`AeadError::NonceLength`] for a nonce of the wrong length,
    /// [`AeadError::MessageTooLong`] for a message beyond the counter range.
    fn seal(&self, nonce: &[u8], aad: &[u8], in_out: &mut [u8]) -> Result<Tag, AeadError>;

    /// Verifies `tag` over `in_out` and `aad` and, only then, decrypts
    /// `in_out` in place.
    ///
    /// # Errors
    ///
    /// [`AeadError::NonceLength`] for a nonce of the wrong length,
    /// [`AeadError::MessageTooLong`] for a message beyond the counter range,
    /// [`AeadError::BadTag`] when verification fails, in which case `in_out`
    /// has been cleared.
    fn open(&self, nonce: &[u8], aad: &[u8], in_out: &mut [u8], tag: &Tag)
    -> Result<(), AeadError>;
}

/// The zeros that pad an authenticated part to a multiple of sixteen.
const ZEROS: [u8; 16] = [0u8; 16];

/// The zeros that pad an authenticated part of `len` bytes to a multiple of
/// sixteen.
pub(crate) const fn padding(len: usize) -> &'static [u8] {
    // `len & 15` is below sixteen, so the needed count is below sixteen and
    // the split is inside the array.
    let needed = 16usize.wrapping_sub(len & 15) & 15;
    let (zeros, _) = ZEROS.split_at(needed);
    zeros
}
