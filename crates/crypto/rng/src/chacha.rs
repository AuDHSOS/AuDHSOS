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
//! Invariants: the key is replaced after every run of output; a reseed
//! mixes fresh material into the key rather than replacing it, so a
//! source that turns out to be predictable cannot take the state over;
//! no key produces more than [`RESEED_BYTES`], because a request wider
//! than the budget is served in runs with a reseed between them; and a
//! request whose reseed fails hands the caller no bytes.

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

    /// Writes the keystream of the current key from block one into
    /// `chunk`, then replaces the key.
    ///
    /// `chunk` is at most [`RESEED_BYTES`] long, which is 2^14 blocks, so
    /// the counter stays far below its range of 2^32 and the stream
    /// cannot run out under one key.
    fn produce(&mut self, chunk: &mut [u8]) {
        let cipher = ChaCha20::new(self.key.as_bytes());
        let nonce = self.nonce();
        for (index, block) in chunk.chunks_mut(chacha20::BLOCK_LEN).enumerate() {
            let counter = u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1);
            for (slot, byte) in block.iter_mut().zip(cipher.block(&nonce, counter)) {
                *slot = byte;
            }
        }
        self.rekey();
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
    /// A request wider than what the budget still allows is served in
    /// runs: each run takes at most the remaining budget from one key,
    /// and the generator reseeds between two runs. A request of `n`
    /// bytes therefore costs at most `n / RESEED_BYTES` reseeds, rounded
    /// up, and no key ever produces more than [`RESEED_BYTES`].
    ///
    /// A reseed that fails wipes the bytes the request has produced so
    /// far and leaves the rest of `out` as it was, so a refusal hands the
    /// caller no bytes.
    fn fill(&mut self, out: &mut [u8]) -> Result<(), RngError> {
        let mut done = 0usize;
        while done < out.len() {
            let (produced, rest) = out.split_at_mut(done);
            if self.budget == 0
                && let Err(error) = self.reseed()
            {
                wipe(produced);
                return Err(error);
            }
            let take = usize::try_from(self.budget)
                .unwrap_or(usize::MAX)
                .min(rest.len());
            let (chunk, _) = rest.split_at_mut(take);
            self.produce(chunk);
            self.budget = self
                .budget
                .saturating_sub(u64::try_from(take).unwrap_or(u64::MAX));
            done = done.saturating_add(take);
        }
        Ok(())
    }
}
