// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What every hash function in this crate provides.
//!
//! Invariant of every implementation: feeding a message in any sequence of
//! chunks produces the digest of the whole message. The state carries a
//! partial block and the number of bytes seen; nothing else.

/// A hash function with a fixed block and output length.
pub trait Hash: Clone + Sized {
    /// Bytes the compression function consumes at a time.
    const BLOCK_LEN: usize;
    /// Bytes of digest.
    const OUTPUT_LEN: usize;
    /// A block of zeros, in the concrete block type. HMAC needs a buffer of
    /// exactly one block and cannot name `[u8; Self::BLOCK_LEN]`.
    const ZERO_BLOCK: Self::Block;

    /// The digest.
    type Output: AsRef<[u8]> + Copy;
    /// One block of input.
    type Block: AsRef<[u8]> + AsMut<[u8]> + Copy;

    /// A state that has seen no input.
    fn new() -> Self;

    /// Adds `bytes` to the message.
    fn update(&mut self, bytes: &[u8]);

    /// Pads the message and returns the digest.
    fn finish(self) -> Self::Output;

    /// The digest of one contiguous message.
    #[must_use]
    fn digest(bytes: &[u8]) -> Self::Output {
        let mut state = Self::new();
        state.update(bytes);
        state.finish()
    }
}
