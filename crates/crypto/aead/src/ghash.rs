// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! GHASH from NIST SP 800-38D, the universal hash GCM authenticates with.
//!
//! Multiplication happens in GF(2^128) with the bit order of the standard:
//! the first bit of a block is the most significant. The usual
//! implementation precomputes a table of products of the hash key, which
//! makes every message byte select a table row; this one is masked
//! shift-and-exclusive-or over `u64` words instead, so no memory access
//! depends on the key or on the message.
//!
//! The hash key and the accumulator are held bit-reversed, where bit `j`
//! is the coefficient of `X^j`, because that is the order the shifts of a
//! carry-less multiplication assume. [`GHash::new`] reverses the key, the
//! private `block` reverses each input block, and [`GHash::finish`]
//! reverses the result back into the order of the standard.
//!
//! One product costs three 64-bit carry-less multiplications (Karatsuba)
//! of 64 masked steps each over `u64` words, and one fixed reduction,
//! against the 128 steps over `u128` a bit-at-a-time shift-and-reduce
//! loop takes.
//!
//! Invariants: `buffered` is smaller than `BLOCK_LEN` between calls; a
//! caller that feeds the parts GCM prescribes, each padded to a block,
//! never leaves a partial block behind; the hash key and the accumulator
//! are overwritten when the computation goes out of scope.

use crypto_ct::wipe;

/// Bytes the hash consumes at a time.
pub const BLOCK_LEN: usize = 16;

/// A GHASH computation under one hash key.
#[derive(Clone)]
pub struct GHash {
    /// The hash key, the encryption of the zero block, bit-reversed.
    key: u128,
    /// The accumulator, bit-reversed.
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
            key: u128::from_be_bytes(*key).reverse_bits(),
            accumulator: 0,
            buffer: [0u8; BLOCK_LEN],
            buffered: 0,
        }
    }

    /// Overwrites the hash key, the accumulator, and the partial block with
    /// zeros. `Drop` calls this; a cleared computation hashes under the
    /// all-zero key and is for dropping only.
    pub(crate) fn clear(&mut self) {
        self.key = 0;
        self.accumulator = 0;
        self.buffered = 0;
        wipe(&mut self.buffer);
        let _ = core::hint::black_box(&self.key);
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
        self.accumulator.reverse_bits().to_be_bytes()
    }

    /// Adds one block: exclusive-or into the accumulator, then multiply by
    /// the hash key. The block is reversed into the order the accumulator
    /// holds.
    fn block(&mut self, block: &[u8; BLOCK_LEN]) {
        self.accumulator ^= u128::from_be_bytes(*block).reverse_bits();
        self.accumulator = multiply(self.accumulator, self.key);
    }
}

impl Drop for GHash {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Multiplication in GF(2^128) over operands whose bit `j` is the
/// coefficient of `X^j`: the carry-less product of the two, then the
/// reduction modulo the polynomial of the field.
fn multiply(x: u128, y: u128) -> u128 {
    let (high, low) = carryless(x, y);
    reduce(high, low)
}

/// The 256-bit carry-less product, high half first, by Karatsuba over the
/// 64-bit halves: three products where the schoolbook form takes four.
fn carryless(x: u128, y: u128) -> (u128, u128) {
    let (x_low, x_high) = halves(x);
    let (y_low, y_high) = halves(y);

    let low = carryless64(x_low, y_low);
    let high = carryless64(x_high, y_high);
    let middle = carryless64(x_low ^ x_high, y_low ^ y_high) ^ low ^ high;

    (
        high ^ middle.wrapping_shr(64),
        low ^ middle.wrapping_shl(64),
    )
}

/// The least significant and the most significant 64 bits. Both halves fit
/// a `u64` by construction, so the fallback of the conversion is dead.
fn halves(value: u128) -> (u64, u64) {
    let low = u64::try_from(value & u128::from(u64::MAX)).unwrap_or(0);
    let high = u64::try_from(value.wrapping_shr(64)).unwrap_or(0);
    (low, high)
}

/// The 128-bit carry-less product of two 64-bit values: one masked
/// shift-and-exclusive-or step per bit of `y`, over `u64` words, which is
/// what keeps the step off the `u128` shift path.
///
/// The step for bit zero stands outside the loop because it alone shifts
/// nothing out of the low word, which keeps the shift count of the other
/// 63 steps within the width of the word.
fn carryless64(x: u64, y: u64) -> u128 {
    let mut low = x & 0u64.wrapping_sub(y & 1);
    let mut high = 0u64;
    for shift in 1..64u32 {
        let selected = 0u64.wrapping_sub(y.wrapping_shr(shift) & 1);
        low ^= x.wrapping_shl(shift) & selected;
        high ^= x.wrapping_shr(64u32.wrapping_sub(shift)) & selected;
    }
    u128::from(high).wrapping_shl(64) | u128::from(low)
}

/// The 256-bit value `high * X^128 + low` modulo
/// `X^128 + X^7 + X^2 + X + 1`.
///
/// `X^128` is congruent to `X^7 + X^2 + X + 1`, so folding `high` down
/// leaves a remainder of degree at most six above the 128th bit; folding
/// that remainder down a second time stays below the 128th bit and ends
/// the reduction.
const fn reduce(high: u128, low: u128) -> u128 {
    let folded = high ^ high.wrapping_shl(1) ^ high.wrapping_shl(2) ^ high.wrapping_shl(7);
    let carried = high.wrapping_shr(127) ^ high.wrapping_shr(126) ^ high.wrapping_shr(121);
    let twice =
        carried ^ carried.wrapping_shl(1) ^ carried.wrapping_shl(2) ^ carried.wrapping_shl(7);
    low ^ folded ^ twice
}
