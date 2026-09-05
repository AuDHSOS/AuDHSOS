// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `Poly1305` from RFC 8439.
//!
//! The accumulator is an integer modulo 2^130 - 5, held in three limbs of
//! 44, 44, and 42 bits so that a limb product fits in 128 bits. Every step
//! is unconditional arithmetic: the final reduction picks between two
//! candidates with a mask rather than with a branch, so the running time
//! depends on the message length alone.
//!
//! Invariants: `buffered` is smaller than `BLOCK_LEN` between calls; the
//! limbs of the accumulator stay within their widths after every block.

use crypto_ct::{Choice, ct_eq};

use crate::aead::{TAG_LEN, Tag};

/// Bytes of key: the multiplier and the addend, in that order.
pub const KEY_LEN: usize = 32;
/// Bytes the accumulator consumes at a time.
pub const BLOCK_LEN: usize = 16;

/// The width of the two lower limbs.
const MASK_44: u64 = 0x0fff_ffff_ffff;
/// The width of the highest limb.
const MASK_42: u64 = 0x03ff_ffff_ffff;
/// The bit a full block sets: 2^128 in the limb representation.
const HIGH_BIT: u64 = 1 << 40;

/// A `Poly1305` computation.
///
/// The multiplier and the addend are key material. They are cleared when
/// the value is dropped.
#[derive(Clone)]
pub struct Poly1305 {
    /// The clamped multiplier, in three limbs.
    r: [u64; 3],
    /// The addend of the last step, as two little-endian words.
    pad: [u64; 2],
    /// The accumulator, in three limbs.
    h: [u64; 3],
    /// The partial block.
    buffer: [u8; BLOCK_LEN],
    /// Bytes held in `buffer`, always below `BLOCK_LEN`.
    buffered: usize,
}

impl Poly1305 {
    /// A computation under `key`.
    ///
    /// The lower half of the key is clamped as RFC 8439 prescribes: the
    /// four bytes that would carry the top bits of a limb are masked, and
    /// three bytes lose their two lowest bits.
    #[must_use]
    pub fn new(key: &[u8; KEY_LEN]) -> Poly1305 {
        let (halves, _) = key.as_chunks::<16>();
        let mut multiplier = [0u8; 16];
        let mut addend = [0u8; 16];
        for (slot, byte) in multiplier
            .iter_mut()
            .zip(halves.first().into_iter().flatten())
        {
            *slot = *byte;
        }
        for (slot, byte) in addend.iter_mut().zip(halves.get(1).into_iter().flatten()) {
            *slot = *byte;
        }

        multiplier[3] &= 0x0F;
        multiplier[7] &= 0x0F;
        multiplier[11] &= 0x0F;
        multiplier[15] &= 0x0F;
        multiplier[4] &= 0xFC;
        multiplier[8] &= 0xFC;
        multiplier[12] &= 0xFC;

        let (low, high) = words(&multiplier);
        let r = [
            low & MASK_44,
            ((low >> 44) | (high << 20)) & MASK_44,
            (high >> 24) & MASK_42,
        ];
        let (pad_low, pad_high) = words(&addend);

        Poly1305 {
            r,
            pad: [pad_low, pad_high],
            h: [0; 3],
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
            self.block(&block, HIGH_BIT);
            self.buffered = 0;
        }

        let (full, remainder) = rest.as_chunks::<BLOCK_LEN>();
        for block in full {
            self.block(block, HIGH_BIT);
        }
        for (slot, byte) in self.buffer.iter_mut().zip(remainder) {
            *slot = *byte;
        }
        self.buffered = remainder.len();
    }

    /// The tag over the message.
    #[must_use]
    pub fn finish(mut self) -> Tag {
        if self.buffered != 0 {
            let mut block = [0u8; BLOCK_LEN];
            for (slot, byte) in block.iter_mut().zip(self.buffer.iter().take(self.buffered)) {
                *slot = *byte;
            }
            if let Some(slot) = block.get_mut(self.buffered) {
                *slot = 1;
            }
            self.block(&block, 0);
            self.buffered = 0;
        }
        self.tag()
    }

    /// The tag over one contiguous message.
    #[must_use]
    pub fn tag_of(key: &[u8; KEY_LEN], message: &[u8]) -> Tag {
        let mut state = Poly1305::new(key);
        state.update(message);
        state.finish()
    }

    /// Whether `tag` is the tag of `message` under `key`.
    #[must_use]
    pub fn verify(key: &[u8; KEY_LEN], message: &[u8], tag: &[u8]) -> Choice {
        ct_eq(&Poly1305::tag_of(key, message), tag)
    }

    /// Adds one block to the accumulator and multiplies by the multiplier.
    ///
    /// `high` is the bit above the block: set for a full block, clear for
    /// the padded last one.
    fn block(&mut self, block: &[u8; BLOCK_LEN], high: u64) {
        let (low, top) = words(block);
        let [r0, r1, r2] = self.r;
        let [mut h0, mut h1, mut h2] = self.h;

        h0 = h0.wrapping_add(low & MASK_44);
        h1 = h1.wrapping_add(((low >> 44) | (top << 20)) & MASK_44);
        h2 = h2.wrapping_add(((top >> 24) & MASK_42) | high);

        // The two upper limbs fold back into the lower ones with a factor of
        // 5 for the modulus and 4 for the limb width, hence twenty.
        let s1 = r1.wrapping_mul(20);
        let s2 = r2.wrapping_mul(20);

        let d0 = product(h0, r0)
            .wrapping_add(product(h1, s2))
            .wrapping_add(product(h2, s1));
        let d1 = product(h0, r1)
            .wrapping_add(product(h1, r0))
            .wrapping_add(product(h2, s2));
        let d2 = product(h0, r2)
            .wrapping_add(product(h1, r1))
            .wrapping_add(product(h2, r0));

        let mut carry = low64(d0 >> 44);
        h0 = low64(d0) & MASK_44;
        let d1 = d1.wrapping_add(u128::from(carry));
        carry = low64(d1 >> 44);
        h1 = low64(d1) & MASK_44;
        let d2 = d2.wrapping_add(u128::from(carry));
        carry = low64(d2 >> 42);
        h2 = low64(d2) & MASK_42;
        h0 = h0.wrapping_add(carry.wrapping_mul(5));
        carry = h0 >> 44;
        h0 &= MASK_44;
        h1 = h1.wrapping_add(carry);

        self.h = [h0, h1, h2];
    }

    /// The final reduction and the addition of the addend.
    fn tag(&self) -> Tag {
        let [mut h0, mut h1, mut h2] = self.h;

        let mut carry = h1 >> 44;
        h1 &= MASK_44;
        h2 = h2.wrapping_add(carry);
        carry = h2 >> 42;
        h2 &= MASK_42;
        h0 = h0.wrapping_add(carry.wrapping_mul(5));
        carry = h0 >> 44;
        h0 &= MASK_44;
        h1 = h1.wrapping_add(carry);
        carry = h1 >> 44;
        h1 &= MASK_44;
        h2 = h2.wrapping_add(carry);
        carry = h2 >> 42;
        h2 &= MASK_42;
        h0 = h0.wrapping_add(carry.wrapping_mul(5));
        carry = h0 >> 44;
        h0 &= MASK_44;
        h1 = h1.wrapping_add(carry);

        // The candidate that has the modulus subtracted. Its highest limb
        // borrows exactly when the accumulator was already below it.
        let mut g0 = h0.wrapping_add(5);
        carry = g0 >> 44;
        g0 &= MASK_44;
        let mut g1 = h1.wrapping_add(carry);
        carry = g1 >> 44;
        g1 &= MASK_44;
        let mut g2 = h2.wrapping_add(carry).wrapping_sub(1 << 42);

        // All ones when the subtraction did not borrow, zero otherwise.
        let take = (g2 >> 63).wrapping_sub(1);
        g0 &= take;
        g1 &= take;
        g2 &= take;
        let keep = !take;
        h0 = (h0 & keep) | g0;
        h1 = (h1 & keep) | g1;
        h2 = (h2 & keep) | g2;

        let [pad_low, pad_high] = self.pad;
        h0 = h0.wrapping_add(pad_low & MASK_44);
        carry = h0 >> 44;
        h0 &= MASK_44;
        h1 = h1
            .wrapping_add(((pad_low >> 44) | (pad_high << 20)) & MASK_44)
            .wrapping_add(carry);
        carry = h1 >> 44;
        h1 &= MASK_44;
        h2 = h2
            .wrapping_add((pad_high >> 24) & MASK_42)
            .wrapping_add(carry);
        h2 &= MASK_42;

        let low = h0 | (h1 << 44);
        let high = (h1 >> 20) | (h2 << 24);

        let mut tag = [0u8; TAG_LEN];
        let (chunks, _) = tag.as_chunks_mut::<8>();
        for (chunk, word) in chunks.iter_mut().zip([low, high]) {
            *chunk = word.to_le_bytes();
        }
        tag
    }
}

impl Drop for Poly1305 {
    fn drop(&mut self) {
        self.r = [0; 3];
        self.pad = [0; 2];
        self.h = [0; 3];
        self.buffer = [0u8; BLOCK_LEN];
        let _ = core::hint::black_box(&self.r);
    }
}

/// The two little-endian words of a block.
fn words(block: &[u8; BLOCK_LEN]) -> (u64, u64) {
    let (first, second) = block.split_at(8);
    let mut low = [0u8; 8];
    let mut high = [0u8; 8];
    for (slot, byte) in low.iter_mut().zip(first) {
        *slot = *byte;
    }
    for (slot, byte) in high.iter_mut().zip(second) {
        *slot = *byte;
    }
    (u64::from_le_bytes(low), u64::from_le_bytes(high))
}

/// The product of two limbs, which needs more than sixty-four bits.
#[expect(clippy::as_conversions, reason = "widening casts in a const fn")]
const fn product(a: u64, b: u64) -> u128 {
    (a as u128).wrapping_mul(b as u128)
}

/// The low sixty-four bits of a wide value.
#[expect(clippy::as_conversions, reason = "the mask keeps only the low 64 bits")]
const fn low64(value: u128) -> u64 {
    (value & 0xFFFF_FFFF_FFFF_FFFF) as u64
}
