// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why an authenticated encryption call was refused.

use core::fmt;

/// Why an authenticated encryption call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AeadError {
    /// The key does not have the length the algorithm takes.
    KeyLength,
    /// The nonce does not have the length the algorithm takes.
    NonceLength,
    /// The message needs more counter blocks than the construction has. For
    /// `ChaCha20` the counter is 32 bits wide; for GCM the counter field is.
    MessageTooLong,
    /// Verification failed: the tag does not belong to this ciphertext, this
    /// associated data, this nonce, and this key. The buffer has been
    /// cleared.
    BadTag,
}

impl fmt::Display for AeadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AeadError::KeyLength => f.write_str("the key has the wrong length"),
            AeadError::NonceLength => f.write_str("the nonce has the wrong length"),
            AeadError::MessageTooLong => {
                f.write_str("the message needs more counter blocks than the construction has")
            }
            AeadError::BadTag => f.write_str("the tag does not authenticate this message"),
        }
    }
}
