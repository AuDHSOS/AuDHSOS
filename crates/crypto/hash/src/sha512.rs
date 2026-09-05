// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! SHA-512 and SHA-384 from FIPS 180-4.
//!
//! The two share every step but the initial state and the length of the
//! digest, so they share the core here and differ only in their wrappers.
//!
//! Invariants: `buffered` is smaller than `BLOCK_LEN` between calls;
//! `length` counts the bytes the core has seen. The message schedule is a
//! rolling window of sixteen words, so every index into it is a literal on
//! a fixed-size array.

use crate::hash::Hash;

/// Bytes the compression function consumes at a time.
pub const BLOCK_LEN: usize = 128;
/// Bytes of a SHA-512 digest.
pub const OUTPUT_LEN_512: usize = 64;
/// Bytes of a SHA-384 digest.
pub const OUTPUT_LEN_384: usize = 48;

/// Bytes of length the padding appends.
const LENGTH_LEN: usize = 16;

/// The SHA-512 initial state: the fractional parts of the square roots of
/// the first eight primes.
const INITIAL_512: [u64; 8] = [
    0x6a09_e667_f3bc_c908,
    0xbb67_ae85_84ca_a73b,
    0x3c6e_f372_fe94_f82b,
    0xa54f_f53a_5f1d_36f1,
    0x510e_527f_ade6_82d1,
    0x9b05_688c_2b3e_6c1f,
    0x1f83_d9ab_fb41_bd6b,
    0x5be0_cd19_137e_2179,
];

/// The SHA-384 initial state: the fractional parts of the square roots of
/// the ninth to the sixteenth prime.
const INITIAL_384: [u64; 8] = [
    0xcbbb_9d5d_c105_9ed8,
    0x629a_292a_367c_d507,
    0x9159_015a_3070_dd17,
    0x152f_ecd8_f70e_5939,
    0x6733_2667_ffc0_0b31,
    0x8eb4_4a87_6858_1511,
    0xdb0c_2e0d_64f9_8fa7,
    0x47b5_481d_befa_4fa4,
];

/// The round constants: the fractional parts of the cube roots of the first
/// eighty primes.
const K: [u64; 80] = [
    0x428a_2f98_d728_ae22,
    0x7137_4491_23ef_65cd,
    0xb5c0_fbcf_ec4d_3b2f,
    0xe9b5_dba5_8189_dbbc,
    0x3956_c25b_f348_b538,
    0x59f1_11f1_b605_d019,
    0x923f_82a4_af19_4f9b,
    0xab1c_5ed5_da6d_8118,
    0xd807_aa98_a303_0242,
    0x1283_5b01_4570_6fbe,
    0x2431_85be_4ee4_b28c,
    0x550c_7dc3_d5ff_b4e2,
    0x72be_5d74_f27b_896f,
    0x80de_b1fe_3b16_96b1,
    0x9bdc_06a7_25c7_1235,
    0xc19b_f174_cf69_2694,
    0xe49b_69c1_9ef1_4ad2,
    0xefbe_4786_384f_25e3,
    0x0fc1_9dc6_8b8c_d5b5,
    0x240c_a1cc_77ac_9c65,
    0x2de9_2c6f_592b_0275,
    0x4a74_84aa_6ea6_e483,
    0x5cb0_a9dc_bd41_fbd4,
    0x76f9_88da_8311_53b5,
    0x983e_5152_ee66_dfab,
    0xa831_c66d_2db4_3210,
    0xb003_27c8_98fb_213f,
    0xbf59_7fc7_beef_0ee4,
    0xc6e0_0bf3_3da8_8fc2,
    0xd5a7_9147_930a_a725,
    0x06ca_6351_e003_826f,
    0x1429_2967_0a0e_6e70,
    0x27b7_0a85_46d2_2ffc,
    0x2e1b_2138_5c26_c926,
    0x4d2c_6dfc_5ac4_2aed,
    0x5338_0d13_9d95_b3df,
    0x650a_7354_8baf_63de,
    0x766a_0abb_3c77_b2a8,
    0x81c2_c92e_47ed_aee6,
    0x9272_2c85_1482_353b,
    0xa2bf_e8a1_4cf1_0364,
    0xa81a_664b_bc42_3001,
    0xc24b_8b70_d0f8_9791,
    0xc76c_51a3_0654_be30,
    0xd192_e819_d6ef_5218,
    0xd699_0624_5565_a910,
    0xf40e_3585_5771_202a,
    0x106a_a070_32bb_d1b8,
    0x19a4_c116_b8d2_d0c8,
    0x1e37_6c08_5141_ab53,
    0x2748_774c_df8e_eb99,
    0x34b0_bcb5_e19b_48a8,
    0x391c_0cb3_c5c9_5a63,
    0x4ed8_aa4a_e341_8acb,
    0x5b9c_ca4f_7763_e373,
    0x682e_6ff3_d6b2_b8a3,
    0x748f_82ee_5def_b2fc,
    0x78a5_636f_4317_2f60,
    0x84c8_7814_a1f0_ab72,
    0x8cc7_0208_1a64_39ec,
    0x90be_fffa_2363_1e28,
    0xa450_6ceb_de82_bde9,
    0xbef9_a3f7_b2c6_7915,
    0xc671_78f2_e372_532b,
    0xca27_3ece_ea26_619c,
    0xd186_b8c7_21c0_c207,
    0xeada_7dd6_cde0_eb1e,
    0xf57d_4f7f_ee6e_d178,
    0x06f0_67aa_7217_6fba,
    0x0a63_7dc5_a2c8_98a6,
    0x113f_9804_bef9_0dae,
    0x1b71_0b35_131c_471b,
    0x28db_77f5_2304_7d84,
    0x32ca_ab7b_40c7_2493,
    0x3c9e_be0a_15c9_bebc,
    0x431d_67c4_9c10_0d4c,
    0x4cc5_d4be_cb3e_42b6,
    0x597f_299c_fc65_7e2a,
    0x5fcb_6fab_3ad6_faec,
    0x6c44_198c_4a47_5817,
];

/// What SHA-512 and SHA-384 have in common.
#[derive(Clone)]
struct Core {
    /// The eight chaining words.
    state: [u64; 8],
    /// The partial block.
    buffer: [u8; BLOCK_LEN],
    /// Bytes held in `buffer`, always below `BLOCK_LEN`.
    buffered: usize,
    /// Bytes the core has seen.
    length: u64,
}

impl Core {
    /// A core that has seen no input.
    const fn new(initial: [u64; 8]) -> Core {
        Core {
            state: initial,
            buffer: [0u8; BLOCK_LEN],
            buffered: 0,
            length: 0,
        }
    }

    /// Adds `bytes` to the message.
    fn update(&mut self, bytes: &[u8]) {
        let added = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.length = self.length.wrapping_add(added);
        self.absorb(bytes);
    }

    /// Pads the message and returns the full digest.
    fn finish(mut self) -> [u8; OUTPUT_LEN_512] {
        let bits = u128::from(self.length).wrapping_mul(8);
        self.absorb(&[0x80]);
        while self.buffered != BLOCK_LEN.wrapping_sub(LENGTH_LEN) {
            self.absorb(&[0x00]);
        }
        self.absorb(&bits.to_be_bytes());
        let mut digest = [0u8; OUTPUT_LEN_512];
        let (chunks, _) = digest.as_chunks_mut::<8>();
        for (chunk, word) in chunks.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        digest
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

/// The state of a SHA-512 computation.
#[derive(Clone)]
pub struct Sha512 {
    /// The shared core, started from the SHA-512 initial state.
    core: Core,
}

impl Sha512 {
    /// A state that has seen no input.
    #[must_use]
    pub const fn new() -> Sha512 {
        Sha512 {
            core: Core::new(INITIAL_512),
        }
    }

    /// Adds `bytes` to the message.
    pub fn update(&mut self, bytes: &[u8]) {
        self.core.update(bytes);
    }

    /// Pads the message and returns the digest.
    #[must_use]
    pub fn finish(self) -> [u8; OUTPUT_LEN_512] {
        self.core.finish()
    }

    /// The digest of one contiguous message.
    #[must_use]
    pub fn digest(bytes: &[u8]) -> [u8; OUTPUT_LEN_512] {
        let mut state = Sha512::new();
        state.update(bytes);
        state.finish()
    }
}

impl Default for Sha512 {
    fn default() -> Sha512 {
        Sha512::new()
    }
}

impl Hash for Sha512 {
    const BLOCK_LEN: usize = BLOCK_LEN;
    const OUTPUT_LEN: usize = OUTPUT_LEN_512;
    const ZERO_BLOCK: [u8; BLOCK_LEN] = [0u8; BLOCK_LEN];

    type Output = [u8; OUTPUT_LEN_512];
    type Block = [u8; BLOCK_LEN];

    fn new() -> Sha512 {
        Sha512::new()
    }

    fn update(&mut self, bytes: &[u8]) {
        Sha512::update(self, bytes);
    }

    fn finish(self) -> [u8; OUTPUT_LEN_512] {
        Sha512::finish(self)
    }
}

/// The state of a SHA-384 computation.
#[derive(Clone)]
pub struct Sha384 {
    /// The shared core, started from the SHA-384 initial state.
    core: Core,
}

impl Sha384 {
    /// A state that has seen no input.
    #[must_use]
    pub const fn new() -> Sha384 {
        Sha384 {
            core: Core::new(INITIAL_384),
        }
    }

    /// Adds `bytes` to the message.
    pub fn update(&mut self, bytes: &[u8]) {
        self.core.update(bytes);
    }

    /// Pads the message and returns the digest: the first forty-eight bytes
    /// of the core's output.
    #[must_use]
    pub fn finish(self) -> [u8; OUTPUT_LEN_384] {
        let full = self.core.finish();
        let mut digest = [0u8; OUTPUT_LEN_384];
        for (slot, byte) in digest.iter_mut().zip(full) {
            *slot = byte;
        }
        digest
    }

    /// The digest of one contiguous message.
    #[must_use]
    pub fn digest(bytes: &[u8]) -> [u8; OUTPUT_LEN_384] {
        let mut state = Sha384::new();
        state.update(bytes);
        state.finish()
    }
}

impl Default for Sha384 {
    fn default() -> Sha384 {
        Sha384::new()
    }
}

impl Hash for Sha384 {
    const BLOCK_LEN: usize = BLOCK_LEN;
    const OUTPUT_LEN: usize = OUTPUT_LEN_384;
    const ZERO_BLOCK: [u8; BLOCK_LEN] = [0u8; BLOCK_LEN];

    type Output = [u8; OUTPUT_LEN_384];
    type Block = [u8; BLOCK_LEN];

    fn new() -> Sha384 {
        Sha384::new()
    }

    fn update(&mut self, bytes: &[u8]) {
        Sha384::update(self, bytes);
    }

    fn finish(self) -> [u8; OUTPUT_LEN_384] {
        Sha384::finish(self)
    }
}

/// The sixteen words of one block, big-endian.
fn words(block: &[u8; BLOCK_LEN]) -> [u64; 16] {
    let mut schedule = [0u64; 16];
    let (chunks, _) = block.as_chunks::<8>();
    for (word, chunk) in schedule.iter_mut().zip(chunks) {
        *word = u64::from_be_bytes(*chunk);
    }
    schedule
}

/// Advances the rolling window by sixteen words.
///
/// The window holds `W[i-16]` to `W[i-1]`. Each assignment turns the oldest
/// word into the newest one, so after sixteen assignments the window holds
/// the next sixteen words of the schedule.
const fn expand(w: &mut [u64; 16]) {
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
fn rounds(working: &mut [u64; 8], schedule: &[u64; 16], constants: &[u64; 16]) {
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
const fn choose(x: u64, y: u64, z: u64) -> u64 {
    (x & y) ^ (!x & z)
}

/// The bit the majority of the three arguments carries.
const fn majority(x: u64, y: u64, z: u64) -> u64 {
    (x & y) ^ (x & z) ^ (y & z)
}

/// The first of the two functions of the working state.
const fn big_sigma0(x: u64) -> u64 {
    x.rotate_right(28) ^ x.rotate_right(34) ^ x.rotate_right(39)
}

/// The second of the two functions of the working state.
const fn big_sigma1(x: u64) -> u64 {
    x.rotate_right(14) ^ x.rotate_right(18) ^ x.rotate_right(41)
}

/// The first of the two functions of the message schedule.
const fn sigma0(x: u64) -> u64 {
    x.rotate_right(1) ^ x.rotate_right(8) ^ x.wrapping_shr(7)
}

/// The second of the two functions of the message schedule.
const fn sigma1(x: u64) -> u64 {
    x.rotate_right(19) ^ x.rotate_right(61) ^ x.wrapping_shr(6)
}
