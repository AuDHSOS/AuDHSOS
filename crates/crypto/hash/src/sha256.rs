// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! SHA-256 from FIPS 180-4.
//!
//! Invariants: `buffered` is smaller than `BLOCK_LEN` between calls;
//! `length` counts the bytes the state has seen. The message schedule is a
//! rolling window of sixteen words, so every index into it is a literal on
//! a fixed-size array and no access can leave the array.

use crate::hash::Hash;

/// Bytes the compression function consumes at a time.
pub const BLOCK_LEN: usize = 64;
/// Bytes of digest.
pub const OUTPUT_LEN: usize = 32;

/// Bytes of length the padding appends.
const LENGTH_LEN: usize = 8;

/// The initial state: the fractional parts of the square roots of the first
/// eight primes.
const INITIAL: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

/// The round constants: the fractional parts of the cube roots of the first
/// sixty-four primes.
const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// The state of a SHA-256 computation.
#[derive(Clone)]
pub struct Sha256 {
    /// The eight chaining words.
    state: [u32; 8],
    /// The partial block.
    buffer: [u8; BLOCK_LEN],
    /// Bytes held in `buffer`, always below `BLOCK_LEN`.
    buffered: usize,
    /// Bytes the state has seen.
    length: u64,
}

impl Sha256 {
    /// A state that has seen no input.
    #[must_use]
    pub const fn new() -> Sha256 {
        Sha256 {
            state: INITIAL,
            buffer: [0u8; BLOCK_LEN],
            buffered: 0,
            length: 0,
        }
    }

    /// Adds `bytes` to the message.
    pub fn update(&mut self, bytes: &[u8]) {
        let added = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.length = self.length.wrapping_add(added);
        self.absorb(bytes);
    }

    /// Pads the message and returns the digest.
    #[must_use]
    pub fn finish(mut self) -> [u8; OUTPUT_LEN] {
        let bits = self.length.wrapping_mul(8);
        self.absorb(&[0x80]);
        while self.buffered != BLOCK_LEN.wrapping_sub(LENGTH_LEN) {
            self.absorb(&[0x00]);
        }
        self.absorb(&bits.to_be_bytes());
        let mut digest = [0u8; OUTPUT_LEN];
        let (chunks, _) = digest.as_chunks_mut::<4>();
        for (chunk, word) in chunks.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        digest
    }

    /// The digest of one contiguous message.
    #[must_use]
    pub fn digest(bytes: &[u8]) -> [u8; OUTPUT_LEN] {
        let mut state = Sha256::new();
        state.update(bytes);
        state.finish()
    }

    /// Consumes `bytes` without counting them, which is what the padding
    /// needs: its bytes are part of the block stream but not of the message.
    fn absorb(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
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
            self.compress(&block);
            self.buffered = 0;
        }
        let (blocks, remainder) = rest.as_chunks::<BLOCK_LEN>();
        for block in blocks {
            self.compress(block);
        }
        for (slot, byte) in self.buffer.iter_mut().zip(remainder) {
            *slot = *byte;
        }
        self.buffered = remainder.len();
    }

    /// One application of the compression function.
    fn compress(&mut self, block: &[u8; BLOCK_LEN]) {
        let mut schedule = words(block);
        let mut working = self.state;
        let (constants, _) = K.as_chunks::<16>();
        let mut groups = constants.iter();
        if let Some(first) = groups.next() {
            rounds(&mut working, &schedule, first);
        }
        for group in groups {
            expand(&mut schedule);
            rounds(&mut working, &schedule, group);
        }
        for (slot, value) in self.state.iter_mut().zip(working) {
            *slot = slot.wrapping_add(value);
        }
    }
}

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

impl Hash for Sha256 {
    const BLOCK_LEN: usize = BLOCK_LEN;
    const OUTPUT_LEN: usize = OUTPUT_LEN;
    const ZERO_BLOCK: [u8; BLOCK_LEN] = [0u8; BLOCK_LEN];

    type Output = [u8; OUTPUT_LEN];
    type Block = [u8; BLOCK_LEN];

    fn new() -> Sha256 {
        Sha256::new()
    }

    fn update(&mut self, bytes: &[u8]) {
        Sha256::update(self, bytes);
    }

    fn finish(self) -> [u8; OUTPUT_LEN] {
        Sha256::finish(self)
    }
}

/// The sixteen words of one block, big-endian.
fn words(block: &[u8; BLOCK_LEN]) -> [u32; 16] {
    let mut schedule = [0u32; 16];
    let (chunks, _) = block.as_chunks::<4>();
    for (word, chunk) in schedule.iter_mut().zip(chunks) {
        *word = u32::from_be_bytes(*chunk);
    }
    schedule
}

/// Advances the rolling window by sixteen words.
///
/// The window holds `W[i-16]` to `W[i-1]`. Each assignment turns the oldest
/// word into the newest one, so after sixteen assignments the window holds
/// the next sixteen words of the schedule.
const fn expand(w: &mut [u32; 16]) {
    w[0] = w[0]
        .wrapping_add(sigma0(w[1]))
        .wrapping_add(w[9])
        .wrapping_add(sigma1(w[14]));
    w[1] = w[1]
        .wrapping_add(sigma0(w[2]))
        .wrapping_add(w[10])
        .wrapping_add(sigma1(w[15]));
    w[2] = w[2]
        .wrapping_add(sigma0(w[3]))
        .wrapping_add(w[11])
        .wrapping_add(sigma1(w[0]));
    w[3] = w[3]
        .wrapping_add(sigma0(w[4]))
        .wrapping_add(w[12])
        .wrapping_add(sigma1(w[1]));
    w[4] = w[4]
        .wrapping_add(sigma0(w[5]))
        .wrapping_add(w[13])
        .wrapping_add(sigma1(w[2]));
    w[5] = w[5]
        .wrapping_add(sigma0(w[6]))
        .wrapping_add(w[14])
        .wrapping_add(sigma1(w[3]));
    w[6] = w[6]
        .wrapping_add(sigma0(w[7]))
        .wrapping_add(w[15])
        .wrapping_add(sigma1(w[4]));
    w[7] = w[7]
        .wrapping_add(sigma0(w[8]))
        .wrapping_add(w[0])
        .wrapping_add(sigma1(w[5]));
    w[8] = w[8]
        .wrapping_add(sigma0(w[9]))
        .wrapping_add(w[1])
        .wrapping_add(sigma1(w[6]));
    w[9] = w[9]
        .wrapping_add(sigma0(w[10]))
        .wrapping_add(w[2])
        .wrapping_add(sigma1(w[7]));
    w[10] = w[10]
        .wrapping_add(sigma0(w[11]))
        .wrapping_add(w[3])
        .wrapping_add(sigma1(w[8]));
    w[11] = w[11]
        .wrapping_add(sigma0(w[12]))
        .wrapping_add(w[4])
        .wrapping_add(sigma1(w[9]));
    w[12] = w[12]
        .wrapping_add(sigma0(w[13]))
        .wrapping_add(w[5])
        .wrapping_add(sigma1(w[10]));
    w[13] = w[13]
        .wrapping_add(sigma0(w[14]))
        .wrapping_add(w[6])
        .wrapping_add(sigma1(w[11]));
    w[14] = w[14]
        .wrapping_add(sigma0(w[15]))
        .wrapping_add(w[7])
        .wrapping_add(sigma1(w[12]));
    w[15] = w[15]
        .wrapping_add(sigma0(w[0]))
        .wrapping_add(w[8])
        .wrapping_add(sigma1(w[13]));
}

/// Sixteen rounds over the current window.
#[expect(
    clippy::many_single_char_names,
    reason = "the working variables are named as in FIPS 180-4"
)]
fn rounds(working: &mut [u32; 8], schedule: &[u32; 16], constants: &[u32; 16]) {
    for (&constant, &word) in constants.iter().zip(schedule.iter()) {
        let [a, b, c, d, e, f, g, h] = *working;
        let t1 = h
            .wrapping_add(big_sigma1(e))
            .wrapping_add(choose(e, f, g))
            .wrapping_add(constant)
            .wrapping_add(word);
        let t2 = big_sigma0(a).wrapping_add(majority(a, b, c));
        *working = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
    }
}

/// `z` where `x` has a zero bit, `y` where it has a one bit.
const fn choose(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

/// The bit the majority of the three arguments carries.
const fn majority(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

/// The first of the two functions of the working state.
const fn big_sigma0(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}

/// The second of the two functions of the working state.
const fn big_sigma1(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}

/// The first of the two functions of the message schedule.
const fn sigma0(x: u32) -> u32 {
    x.rotate_right(7) ^ x.rotate_right(18) ^ x.wrapping_shr(3)
}

/// The second of the two functions of the message schedule.
const fn sigma1(x: u32) -> u32 {
    x.rotate_right(17) ^ x.rotate_right(19) ^ x.wrapping_shr(10)
}
