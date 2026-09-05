// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ChaCha20` from RFC 8439.
//!
//! Invariants: the cipher branches on nothing but lengths and the block
//! counter, both public; the counter never wraps, because a message that
//! would need it is refused before a byte is touched.

use crate::error::AeadError;

/// Bytes of key.
pub const KEY_LEN: usize = 32;
/// Bytes of nonce.
pub const NONCE_LEN: usize = 12;
/// Bytes the block function produces at a time.
pub const BLOCK_LEN: usize = 64;

/// The first four state words: "expand 32-byte k" in little-endian.
const CONSTANTS: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

/// The key of a `ChaCha20` stream, as the eight words the state holds.
#[derive(Clone)]
pub struct ChaCha20 {
    /// Words four to eleven of the state.
    key: [u32; 8],
}

impl ChaCha20 {
    /// A cipher under `key`.
    #[must_use]
    pub fn new(key: &[u8; KEY_LEN]) -> ChaCha20 {
        let mut words = [0u32; 8];
        let (chunks, _) = key.as_chunks::<4>();
        for (word, chunk) in words.iter_mut().zip(chunks) {
            *word = u32::from_le_bytes(*chunk);
        }
        ChaCha20 { key: words }
    }

    /// The keystream block for `counter`.
    #[must_use]
    pub fn block(&self, nonce: &[u8; NONCE_LEN], counter: u32) -> [u8; BLOCK_LEN] {
        let start = self.state(nonce, counter);
        let mut working = start;
        for _ in 0..10 {
            double_round(&mut working);
        }
        for (word, initial) in working.iter_mut().zip(start) {
            *word = word.wrapping_add(initial);
        }

        let mut block = [0u8; BLOCK_LEN];
        let (chunks, _) = block.as_chunks_mut::<4>();
        for (chunk, word) in chunks.iter_mut().zip(working) {
            *chunk = word.to_le_bytes();
        }
        block
    }

    /// Exclusive-ors the keystream from `counter` onward into `data`.
    ///
    /// # Errors
    ///
    /// [`AeadError::MessageTooLong`] when `data` needs a counter beyond
    /// `u32::MAX`. Nothing is written in that case.
    pub fn apply_keystream(
        &self,
        nonce: &[u8; NONCE_LEN],
        counter: u32,
        data: &mut [u8],
    ) -> Result<(), AeadError> {
        let blocks = data.len().div_ceil(BLOCK_LEN);
        if let Some(last) = blocks.checked_sub(1) {
            let last = u32::try_from(last).map_err(|_| AeadError::MessageTooLong)?;
            counter.checked_add(last).ok_or(AeadError::MessageTooLong)?;
        }

        let mut position = counter;
        let (full, remainder) = data.as_chunks_mut::<BLOCK_LEN>();
        for block in full {
            let keystream = self.block(nonce, position);
            for (byte, key) in block.iter_mut().zip(keystream) {
                *byte ^= key;
            }
            position = position.wrapping_add(1);
        }
        if !remainder.is_empty() {
            let keystream = self.block(nonce, position);
            for (byte, key) in remainder.iter_mut().zip(keystream) {
                *byte ^= key;
            }
        }
        Ok(())
    }

    /// The initial state: the constants, the key, the counter, the nonce.
    fn state(&self, nonce: &[u8; NONCE_LEN], counter: u32) -> [u32; 16] {
        let mut state = [0u32; 16];
        for (slot, value) in state.iter_mut().zip(CONSTANTS) {
            *slot = value;
        }
        for (slot, value) in state.iter_mut().skip(4).zip(self.key) {
            *slot = value;
        }
        state[12] = counter;
        let (chunks, _) = nonce.as_chunks::<4>();
        for (slot, chunk) in state.iter_mut().skip(13).zip(chunks) {
            *slot = u32::from_le_bytes(*chunk);
        }
        state
    }
}

/// The four column rounds and the four diagonal rounds of one double round.
const fn double_round(s: &mut [u32; 16]) {
    (s[0], s[4], s[8], s[12]) = quarter(s[0], s[4], s[8], s[12]);
    (s[1], s[5], s[9], s[13]) = quarter(s[1], s[5], s[9], s[13]);
    (s[2], s[6], s[10], s[14]) = quarter(s[2], s[6], s[10], s[14]);
    (s[3], s[7], s[11], s[15]) = quarter(s[3], s[7], s[11], s[15]);
    (s[0], s[5], s[10], s[15]) = quarter(s[0], s[5], s[10], s[15]);
    (s[1], s[6], s[11], s[12]) = quarter(s[1], s[6], s[11], s[12]);
    (s[2], s[7], s[8], s[13]) = quarter(s[2], s[7], s[8], s[13]);
    (s[3], s[4], s[9], s[14]) = quarter(s[3], s[4], s[9], s[14]);
}

/// The quarter round, taking and returning its four words rather than
/// indexing the state with a variable.
const fn quarter(a: u32, b: u32, c: u32, d: u32) -> (u32, u32, u32, u32) {
    let a = a.wrapping_add(b);
    let d = (d ^ a).rotate_left(16);
    let c = c.wrapping_add(d);
    let b = (b ^ c).rotate_left(12);
    let a = a.wrapping_add(b);
    let d = (d ^ a).rotate_left(8);
    let c = c.wrapping_add(d);
    let b = (b ^ c).rotate_left(7);
    (a, b, c, d)
}
