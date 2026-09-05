// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The running hash of the handshake.
//!
//! Which hash it is depends on the cipher suite, and the suite is not
//! known until the server has answered. Rather than buffer the messages
//! until then, this keeps both hashes and hands out the one that is asked
//! for. The second hash costs a few kilobytes of hashing per handshake and
//! removes a buffer, a length limit, and the failure that comes with them.
//!
//! Invariant: every handshake message is fed exactly once, in the order it
//! appeared on the wire, with its four-byte header included.

use crypto_hash::{Sha256, Sha384};

use crate::error::TlsError;
use crate::secret::Secret;
use crate::suite::CipherSuite;

/// The handshake type that stands for a hashed `ClientHello`.
const MESSAGE_HASH: u8 = 254;

/// The running hash of the handshake, in both lengths.
#[derive(Clone)]
pub struct Transcript {
    /// The transcript under SHA-256.
    short: Sha256,
    /// The transcript under SHA-384.
    long: Sha384,
}

impl Transcript {
    /// A transcript that has seen nothing.
    #[must_use]
    pub const fn new() -> Transcript {
        Transcript {
            short: Sha256::new(),
            long: Sha384::new(),
        }
    }

    /// Adds a handshake message, header included.
    pub fn update(&mut self, message: &[u8]) {
        self.short.update(message);
        self.long.update(message);
    }

    /// The transcript hash under `suite`.
    #[must_use]
    pub fn hash(&self, suite: CipherSuite) -> Secret {
        match suite {
            CipherSuite::Aes256GcmSha384 => Secret::from_slice(self.long.clone().finish().as_ref()),
            _ => Secret::from_slice(self.short.clone().finish().as_ref()),
        }
    }

    /// Replaces what has been seen so far with the hash of it, as a
    /// `HelloRetryRequest` requires.
    ///
    /// The standard does this so that the transcript of a retried
    /// handshake does not depend on the first `ClientHello` twice. Each
    /// hash gets the substitution its own length prescribes, so both
    /// remain usable afterwards.
    pub fn replace_with_message_hash(&mut self) {
        let short = self.short.clone().finish();
        let long = self.long.clone().finish();

        self.short = Sha256::new();
        self.short.update(&[MESSAGE_HASH, 0x00, 0x00, 32]);
        self.short.update(&short);

        self.long = Sha384::new();
        self.long.update(&[MESSAGE_HASH, 0x00, 0x00, 48]);
        self.long.update(&long);
    }

    /// The transcript hash under `suite`, as the caller needs it to derive
    /// a secret.
    ///
    /// # Errors
    ///
    /// This cannot fail today; the result is a `Result` so that a future
    /// hash whose length exceeds the secret type does not become a silent
    /// truncation.
    pub fn checked_hash(&self, suite: CipherSuite) -> Result<Secret, TlsError> {
        let hash = self.hash(suite);
        if hash.len() == suite.hash_len() {
            Ok(hash)
        } else {
            Err(TlsError::TranscriptNotStarted)
        }
    }
}

impl Default for Transcript {
    fn default() -> Transcript {
        Transcript::new()
    }
}
