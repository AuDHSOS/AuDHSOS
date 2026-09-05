// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! GHASH from NIST SP 800-38D, the universal hash GCM authenticates with.
//!
//! Multiplication happens in GF(2^128) with the bit order of the standard:
//! the first bit of a block is the most significant. The usual
//! implementation precomputes a table of products of the hash key, which
//! makes every message byte select a table row; this one is a hundred and
//! twenty-eight masked shift-and-exclusive-or steps instead, so no memory
//! access depends on the key or on the message.
//!
//! Invariants: `buffered` is smaller than `BLOCK_LEN` between calls; a
//! caller that feeds the parts GCM prescribes, each padded to a block,
//! never leaves a partial block behind.

/// Bytes the hash consumes at a time.
pub const BLOCK_LEN: usize = 16;

/// The reduction polynomial of GF(2^128) in the bit order of the standard.
const REDUCTION: u128 = 0xE100_0000_0000_0000_0000_0000_0000_0000;

/// A GHASH computation under one hash key.
#[derive(Clone)]
pub struct GHash {
    /// The hash key, the encryption of the zero block.
    key: u128,
    /// The accumulator.
    accumulator: u128,
    /// The partial block.
    buffer: [u8; BLOCK_LEN],
    /// Bytes held in `buffer`, always below `BLOCK_LEN`.
    buffered: usize,
}

impl GHash {
    /// A computation under the hash key `key`.
    #[must_use]
    pub const fn new(key: &[u8; BLOCK_LEN]) -> GHash {
        GHash {
            key: u128::from_be_bytes(*key),
            accumulator: 0,
            buffer: [0u8; BLOCK_LEN],
            buffered: 0,
        }
    }

    /// Adds `data` to the message.
    pub fn update(&mut self, data: &[u8]) {
        let mut rest = data;
        if self.buffered != 0 {
            let free = BLOCK_LEN.saturating_sub(self.buffered);
            let take = free.min(rest.len());
            let (head, tail) = rest.split_at(take);
            for (slot, byte) in self.buffer.iter_mut().skip(self.buffered).zip(head) {
                *slot = *byte;
            }
            self.buffered = self.buffered.wrapping_add(take);
            rest = tail;
            if self.buffered < BLOCK_LEN {
                return;
            }
            let block = self.buffer;
            self.block(&block);
            self.buffered = 0;
        }

        let (full, remainder) = rest.as_chunks::<BLOCK_LEN>();
        for block in full {
            self.block(block);
        }
        for (slot, byte) in self.buffer.iter_mut().zip(remainder) {
            *slot = *byte;
        }
        self.buffered = remainder.len();
    }

    /// The hash over the message. A partial block left over is padded with
    /// zeros; the callers in this crate never leave one.
    #[must_use]
    pub fn finish(mut self) -> [u8; BLOCK_LEN] {
        if self.buffered != 0 {
            let mut block = [0u8; BLOCK_LEN];
            for (slot, byte) in block.iter_mut().zip(self.buffer.iter().take(self.buffered)) {
                *slot = *byte;
            }
            self.block(&block);
            self.buffered = 0;
        }
        self.accumulator.to_be_bytes()
    }

    /// Adds one block: exclusive-or into the accumulator, then multiply by
    /// the hash key.
    fn block(&mut self, block: &[u8; BLOCK_LEN]) {
        self.accumulator ^= u128::from_be_bytes(*block);
        self.accumulator = multiply(self.accumulator, self.key);
    }
}

/// Multiplication in GF(2^128), most significant bit of the first operand
/// first, with the reduction folded in as the running value shifts.
fn multiply(x: u128, y: u128) -> u128 {
    let mut product = 0u128;
    let mut running = y;
    for bit in 0..128u32 {
        let selected = 0u128.wrapping_sub(x.wrapping_shr(127u32.wrapping_sub(bit)) & 1);
        product ^= running & selected;
        let odd = 0u128.wrapping_sub(running & 1);
        running = running.wrapping_shr(1) ^ (REDUCTION & odd);
    }
    product
}
