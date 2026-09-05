// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A deterministic random bit generator over `ChaCha20`.
//!
//! The state is a key and a sequence number. Every request builds the
//! nonce from the sequence, takes its output from block one onwards, and
//! then replaces the key with block zero of the same stream. A state
//! captured after a request therefore says nothing about the bytes the
//! request produced, and the sequence never repeats a nonce under one key
//! because the key changes with it.
//!
//! Invariants: the key is replaced after every request that produces
//! bytes; a reseed mixes fresh material into the key rather than replacing
//! it, so a source that turns out to be predictable cannot take the state
//! over; and no bytes are produced when a due reseed fails.

use crypto_aead::chacha20::{self, ChaCha20};
use crypto_ct::{Secret, wipe};

use crate::error::RngError;
#[cfg(any(test, feature = "test-doubles"))]
use crate::source::SourceCalls;
use crate::source::{Entropy, Rng};

/// Bytes of seed and of key.
pub const KEY_LEN: usize = chacha20::KEY_LEN;

/// Bytes a generator produces before it reseeds. A handshake needs a few
/// hundred, so a mebibyte is a boundary the protocol never approaches and
/// a long-lived connection eventually crosses.
pub const RESEED_BYTES: u64 = 1 << 20;

/// A generator over `ChaCha20` and the source it reseeds from.
pub struct ChaChaRng<E: Entropy> {
    /// The source of fresh material.
    source: E,
    /// The current key.
    key: Secret<KEY_LEN>,
    /// The number of requests served, which is the nonce.
    sequence: u64,
    /// Bytes that may still be produced before a reseed is due.
    budget: u64,
}

impl<E: Entropy> ChaChaRng<E> {
    /// A generator seeded from `source`.
    ///
    /// # Errors
    ///
    /// [`RngError::Entropy`] when the source cannot deliver the seed. No
    /// generator exists in that case, so a caller cannot use one that was
    /// never seeded.
    pub fn new(source: E) -> Result<ChaChaRng<E>, RngError> {
        let mut generator = ChaChaRng {
            source,
            key: Secret::zero(),
            sequence: 0,
            budget: 0,
        };
        generator.reseed()?;
        Ok(generator)
    }

    /// A generator on a known seed, which produces a stream that is a
    /// function of that seed alone until the first reseed.
    ///
    /// This is how the trace tests of the protocol get a handshake they
    /// can compare byte for byte. Product code seeds from a source.
    #[must_use]
    pub const fn from_seed(seed: &[u8; KEY_LEN], source: E) -> ChaChaRng<E> {
        ChaChaRng {
            source,
            key: Secret::new(*seed),
            sequence: 0,
            budget: RESEED_BYTES,
        }
    }

    /// Mixes fresh material from the source into the key and restarts the
    /// budget.
    ///
    /// # Errors
    ///
    /// [`RngError::Entropy`] when the source cannot deliver.
    pub fn reseed(&mut self) -> Result<(), RngError> {
        let mut fresh = [0u8; KEY_LEN];
        self.source.fill(&mut fresh)?;
        for (slot, byte) in self.key.as_bytes_mut().iter_mut().zip(fresh) {
            *slot ^= byte;
        }
        wipe(&mut fresh);
        self.rekey();
        self.budget = RESEED_BYTES;
        Ok(())
    }

    /// How often the entropy source was asked, which is what a test needs
    /// to see that the budget triggers exactly one reseed.
    #[cfg(any(test, feature = "test-doubles"))]
    #[must_use]
    pub fn source_calls(&self) -> usize
    where
        E: SourceCalls,
    {
        self.source.calls()
    }

    /// The nonce of the current request.
    fn nonce(&self) -> [u8; chacha20::NONCE_LEN] {
        let mut nonce = [0u8; chacha20::NONCE_LEN];
        for (slot, byte) in nonce.iter_mut().zip(self.sequence.to_le_bytes()) {
            *slot = byte;
        }
        nonce
    }

    /// Replaces the key with block zero of the current stream and advances
    /// the sequence.
    fn rekey(&mut self) {
        let block = ChaCha20::new(self.key.as_bytes()).block(&self.nonce(), 0);
        for (slot, byte) in self.key.as_bytes_mut().iter_mut().zip(block) {
            *slot = byte;
        }
        self.sequence = self.sequence.wrapping_add(1);
    }
}

impl<E: Entropy> Rng for ChaChaRng<E> {
    fn fill(&mut self, out: &mut [u8]) -> Result<(), RngError> {
        if out.is_empty() {
            return Ok(());
        }
        let wanted = u64::try_from(out.len()).unwrap_or(u64::MAX);
        if wanted > self.budget {
            self.reseed()?;
        }

        out.fill(0);
        let cipher = ChaCha20::new(self.key.as_bytes());
        cipher
            .apply_keystream(&self.nonce(), 1, out)
            .map_err(|_| RngError::Exhausted)?;
        self.rekey();
        self.budget = self.budget.saturating_sub(wanted);
        Ok(())
    }
}
