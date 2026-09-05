// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! AES-128 and AES-256 encryption without a lookup table.
//!
//! The state is bitsliced: eight plane words, one per bit position of a
//! byte, each holding that bit of every byte of four blocks at once. A
//! substitution is then arithmetic in GF(2^8) rather than a table read, so
//! no memory access depends on a key or on plaintext, and four blocks of
//! counter mode are encrypted for the price of one.
//!
//! Only encryption exists. GCM runs the cipher in counter mode and never
//! decrypts a block, so there is no inverse substitution box and no inverse
//! mixing step to get wrong.
//!
//! Invariants: a plane word holds one bit of sixteen bytes in each of four
//! sixteen-bit groups; every permutation of lanes stays inside its group;
//! every index into the state and into the round keys is a literal or comes
//! from an iterator.

/// Blocks that share one set of plane words.
pub const LANES: usize = 4;
/// Bytes of one block.
pub const BLOCK_LEN: usize = 16;

/// The eight bit planes of [`LANES`] blocks.
type Planes = [u64; 8];

/// All planes clear.
const ZERO: Planes = [0; 8];

/// One bit in the same position of every one of the four groups.
const LANE_STEP: u64 = 0x0001_0001_0001_0001;

/// The lane each lane takes its bit from in `ShiftRows`: row `r` moves
/// left by `r` columns.
const SHIFT_ROWS: [usize; 16] = [0, 5, 10, 15, 4, 9, 14, 3, 8, 13, 2, 7, 12, 1, 6, 11];

/// The lanes one row down, for the mixing step.
const ROW_1: [usize; 16] = [1, 2, 3, 0, 5, 6, 7, 4, 9, 10, 11, 8, 13, 14, 15, 12];

/// The lanes two rows down, for the mixing step.
const ROW_2: [usize; 16] = [2, 3, 0, 1, 6, 7, 4, 5, 10, 11, 8, 9, 14, 15, 12, 13];

/// The lanes three rows down, for the mixing step.
const ROW_3: [usize; 16] = [3, 0, 1, 2, 7, 4, 5, 6, 11, 8, 9, 10, 15, 12, 13, 14];

/// An AES key, expanded into round keys in the sliced representation.
#[derive(Clone)]
#[expect(
    clippy::large_enum_variant,
    reason = "the two key schedules are 704 and 960 bytes; a crate without an allocator cannot \
              box the smaller one, and the value is built once per key and never moved again"
)]
pub enum Aes {
    /// A 128-bit key: ten rounds, eleven round keys.
    Aes128([Planes; 11]),
    /// A 256-bit key: fourteen rounds, fifteen round keys.
    Aes256([Planes; 15]),
}

impl Aes {
    /// The cipher under a 128-bit key.
    #[must_use]
    pub fn new_128(key: &[u8; 16]) -> Aes {
        let expanded = expand_128(key);
        let mut keys = [ZERO; 11];
        let (blocks, _) = expanded.as_chunks::<BLOCK_LEN>();
        for (slot, block) in keys.iter_mut().zip(blocks) {
            *slot = slice(&[*block; LANES]);
        }
        Aes::Aes128(keys)
    }

    /// The cipher under a 256-bit key.
    #[must_use]
    pub fn new_256(key: &[u8; 32]) -> Aes {
        let expanded = expand_256(key);
        let mut keys = [ZERO; 15];
        let (blocks, _) = expanded.as_chunks::<BLOCK_LEN>();
        for (slot, block) in keys.iter_mut().zip(blocks) {
            *slot = slice(&[*block; LANES]);
        }
        Aes::Aes256(keys)
    }

    /// Encrypts four blocks.
    pub fn encrypt_blocks(&self, blocks: &mut [[u8; BLOCK_LEN]; LANES]) {
        let mut planes = slice(blocks);
        encrypt_planes(self.round_keys(), &mut planes);
        *blocks = unslice(&planes);
    }

    /// Encrypts one block, leaving the other three lanes unused.
    pub fn encrypt_block(&self, block: &mut [u8; BLOCK_LEN]) {
        let mut blocks = [*block; LANES];
        self.encrypt_blocks(&mut blocks);
        let [first, ..] = blocks;
        *block = first;
    }

    /// The round keys, oldest first.
    const fn round_keys(&self) -> &[Planes] {
        match self {
            Aes::Aes128(keys) => keys,
            Aes::Aes256(keys) => keys,
        }
    }
}

/// The rounds themselves: the first key, then a full round per middle key,
/// then the last round, which has no mixing step.
fn encrypt_planes(round_keys: &[Planes], planes: &mut Planes) {
    let Some((first, rest)) = round_keys.split_first() else {
        return;
    };
    let Some((last, middle)) = rest.split_last() else {
        return;
    };

    add_round_key(planes, first);
    for key in middle {
        sub_bytes(planes);
        shift_rows(planes);
        mix_columns(planes);
        add_round_key(planes, key);
    }
    sub_bytes(planes);
    shift_rows(planes);
    add_round_key(planes, last);
}

/// Exclusive-ors a round key into the state.
fn add_round_key(planes: &mut Planes, key: &Planes) {
    for (plane, word) in planes.iter_mut().zip(key) {
        *plane ^= *word;
    }
}

/// The substitution box: the multiplicative inverse in GF(2^8), with zero
/// mapped to zero, followed by the affine transformation of FIPS 197.
fn sub_bytes(planes: &mut Planes) {
    let inverted = inverse(planes);
    *planes = affine(&inverted);
}

/// `ShiftRows`.
fn shift_rows(planes: &mut Planes) {
    *planes = permute(planes, &SHIFT_ROWS);
}

/// `MixColumns`, written as `b = 2a ^ 3a' ^ a'' ^ a'''` over the rows of a
/// column, with `3a = 2a ^ a`.
fn mix_columns(planes: &mut Planes) {
    let doubled = xtime(planes);
    let doubled_next = permute(&doubled, &ROW_1);
    let next = permute(planes, &ROW_1);
    let over = permute(planes, &ROW_2);
    let across = permute(planes, &ROW_3);

    let mut result = ZERO;
    for (slot, ((((a, b), c), d), e)) in result.iter_mut().zip(
        doubled
            .iter()
            .zip(doubled_next.iter())
            .zip(next.iter())
            .zip(over.iter())
            .zip(across.iter()),
    ) {
        *slot = a ^ b ^ c ^ d ^ e;
    }
    *planes = result;
}

/// Moves every lane of every group to the lane the table names.
fn permute(planes: &Planes, table: &[usize; 16]) -> Planes {
    let mut result = ZERO;
    for (slot, plane) in result.iter_mut().zip(planes) {
        let mut moved = 0u64;
        for (destination, source) in table.iter().enumerate() {
            let bits = plane & (LANE_STEP.wrapping_shl(lane_shift(*source)));
            moved |= if destination >= *source {
                bits.wrapping_shl(lane_shift(destination.wrapping_sub(*source)))
            } else {
                bits.wrapping_shr(lane_shift(source.wrapping_sub(destination)))
            };
        }
        *slot = moved;
    }
    result
}

/// A lane number as a shift amount.
fn lane_shift(lane: usize) -> u32 {
    u32::try_from(lane).unwrap_or(0)
}

/// Doubling in GF(2^8): a shift of the bit planes, with the reduction
/// polynomial exclusive-ored into the planes it touches.
const fn xtime(planes: &Planes) -> Planes {
    let [p0, p1, p2, p3, p4, p5, p6, p7] = *planes;
    [p7, p0 ^ p7, p1, p2 ^ p7, p3 ^ p7, p4, p5, p6]
}

/// The affine transformation of FIPS 197, section 5.1.1.
fn affine(planes: &Planes) -> Planes {
    let [p0, p1, p2, p3, p4, p5, p6, p7] = *planes;
    let constant = 0x63u8;
    let mut result = [
        p0 ^ p4 ^ p5 ^ p6 ^ p7,
        p1 ^ p5 ^ p6 ^ p7 ^ p0,
        p2 ^ p6 ^ p7 ^ p0 ^ p1,
        p3 ^ p7 ^ p0 ^ p1 ^ p2,
        p4 ^ p0 ^ p1 ^ p2 ^ p3,
        p5 ^ p1 ^ p2 ^ p3 ^ p4,
        p6 ^ p2 ^ p3 ^ p4 ^ p5,
        p7 ^ p3 ^ p4 ^ p5 ^ p6,
    ];
    for (bit, plane) in result.iter_mut().enumerate() {
        if constant.wrapping_shr(lane_shift(bit)) & 1 == 1 {
            *plane = !*plane;
        }
    }
    result
}

/// The multiplicative inverse in GF(2^8), which is the two hundred and
/// fifty-fourth power, with zero mapping to zero.
fn inverse(a: &Planes) -> Planes {
    let a2 = square(a);
    let a3 = multiply(&a2, a);
    let a12 = square(&square(&a3));
    let a14 = multiply(&a12, &a2);
    let a15 = multiply(&a12, &a3);
    let a240 = square(&square(&square(&square(&a15))));
    multiply(&a240, &a14)
}

/// Multiplication in GF(2^8) modulo `x^8 + x^4 + x^3 + x + 1`.
fn multiply(a: &Planes, b: &Planes) -> Planes {
    let mut wide = [0u64; 15];
    for (offset, left) in a.iter().enumerate() {
        for (slot, right) in wide.iter_mut().skip(offset).zip(b) {
            *slot ^= left & right;
        }
    }
    reduce(&wide)
}

/// Squaring in GF(2^8), which spreads the bits and reduces.
fn square(a: &Planes) -> Planes {
    let mut wide = [0u64; 15];
    for (slot, plane) in wide.iter_mut().step_by(2).zip(a) {
        *slot = *plane;
    }
    reduce(&wide)
}

/// Folds the terms above degree seven back in with the reduction
/// polynomial, highest first.
fn reduce(wide: &[u64; 15]) -> Planes {
    let mut terms = *wide;
    let high = terms[14];
    terms[10] ^= high;
    terms[9] ^= high;
    terms[7] ^= high;
    terms[6] ^= high;

    let high = terms[13];
    terms[9] ^= high;
    terms[8] ^= high;
    terms[6] ^= high;
    terms[5] ^= high;

    let high = terms[12];
    terms[8] ^= high;
    terms[7] ^= high;
    terms[5] ^= high;
    terms[4] ^= high;

    let high = terms[11];
    terms[7] ^= high;
    terms[6] ^= high;
    terms[4] ^= high;
    terms[3] ^= high;

    let high = terms[10];
    terms[6] ^= high;
    terms[5] ^= high;
    terms[3] ^= high;
    terms[2] ^= high;

    let high = terms[9];
    terms[5] ^= high;
    terms[4] ^= high;
    terms[2] ^= high;
    terms[1] ^= high;

    let high = terms[8];
    terms[4] ^= high;
    terms[3] ^= high;
    terms[1] ^= high;
    terms[0] ^= high;

    let mut result = ZERO;
    for (slot, term) in result.iter_mut().zip(terms) {
        *slot = term;
    }
    result
}

/// Turns four blocks into the eight plane words.
fn slice(blocks: &[[u8; BLOCK_LEN]; LANES]) -> Planes {
    let mut planes = ZERO;
    for (group, block) in blocks.iter().enumerate() {
        let base = group.wrapping_mul(BLOCK_LEN);
        for (lane, byte) in block.iter().enumerate() {
            let shift = lane_shift(base.wrapping_add(lane));
            for (bit, plane) in planes.iter_mut().enumerate() {
                let value = u64::from(byte.wrapping_shr(lane_shift(bit)) & 1);
                *plane |= value.wrapping_shl(shift);
            }
        }
    }
    planes
}

/// Turns the eight plane words back into four blocks.
fn unslice(planes: &Planes) -> [[u8; BLOCK_LEN]; LANES] {
    let mut blocks = [[0u8; BLOCK_LEN]; LANES];
    for (group, block) in blocks.iter_mut().enumerate() {
        let base = group.wrapping_mul(BLOCK_LEN);
        for (lane, byte) in block.iter_mut().enumerate() {
            let shift = lane_shift(base.wrapping_add(lane));
            for (bit, plane) in planes.iter().enumerate() {
                let value = plane.wrapping_shr(shift) & 1;
                let value = u8::try_from(value).unwrap_or(0);
                *byte |= value.wrapping_shl(lane_shift(bit));
            }
        }
    }
    blocks
}

/// The substitution box applied to four bytes, for the key schedule.
pub(crate) fn sub_word(word: [u8; 4]) -> [u8; 4] {
    let mut block = [0u8; BLOCK_LEN];
    for (slot, byte) in block.iter_mut().zip(word) {
        *slot = byte;
    }
    let mut planes = slice(&[block; LANES]);
    sub_bytes(&mut planes);
    let blocks = unslice(&planes);
    let mut result = [0u8; 4];
    for (slot, byte) in result.iter_mut().zip(blocks.first().into_iter().flatten()) {
        *slot = *byte;
    }
    result
}

/// The word rotated by one byte, as the key schedule needs it.
const fn rotate_word(word: [u8; 4]) -> [u8; 4] {
    let [a, b, c, d] = word;
    [b, c, d, a]
}

/// Two words exclusive-ored.
fn xor_word(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    let mut result = [0u8; 4];
    for (slot, (left, right)) in result.iter_mut().zip(a.iter().zip(b.iter())) {
        *slot = left ^ right;
    }
    result
}

/// Doubling of one byte in GF(2^8), for the round constants.
const fn xtime_byte(value: u8) -> u8 {
    let high = 0u8.wrapping_sub(value.wrapping_shr(7));
    value.wrapping_shl(1) ^ (0x1B & high)
}

/// The key schedule of AES-128: forty-four words.
fn expand_128(key: &[u8; 16]) -> [u8; 176] {
    let mut expanded = [0u8; 176];
    let (words, _) = expanded.as_chunks_mut::<4>();
    let (key_words, _) = key.as_chunks::<4>();
    let mut window = [[0u8; 4]; 4];
    for (slot, chunk) in window.iter_mut().zip(key_words) {
        *slot = *chunk;
    }

    let mut iter = words.iter_mut();
    for chunk in key_words {
        if let Some(slot) = iter.next() {
            *slot = *chunk;
        }
    }

    let mut index = 4usize;
    let mut rcon = 1u8;
    for slot in iter {
        let mut temp = window[3];
        if index.is_multiple_of(4) {
            temp = sub_word(rotate_word(temp));
            temp[0] ^= rcon;
            rcon = xtime_byte(rcon);
        }
        let new = xor_word(window[0], temp);
        *slot = new;
        window = [window[1], window[2], window[3], new];
        index = index.wrapping_add(1);
    }
    expanded
}

/// The key schedule of AES-256: sixty words, with a substitution in the
/// middle of every group of eight as well as at its start.
fn expand_256(key: &[u8; 32]) -> [u8; 240] {
    let mut expanded = [0u8; 240];
    let (words, _) = expanded.as_chunks_mut::<4>();
    let (key_words, _) = key.as_chunks::<4>();
    let mut window = [[0u8; 4]; 8];
    for (slot, chunk) in window.iter_mut().zip(key_words) {
        *slot = *chunk;
    }

    let mut iter = words.iter_mut();
    for chunk in key_words {
        if let Some(slot) = iter.next() {
            *slot = *chunk;
        }
    }

    let mut index = 8usize;
    let mut rcon = 1u8;
    for slot in iter {
        let mut temp = window[7];
        if index.is_multiple_of(8) {
            temp = sub_word(rotate_word(temp));
            temp[0] ^= rcon;
            rcon = xtime_byte(rcon);
        } else if index & 7 == 4 {
            temp = sub_word(temp);
        }
        let new = xor_word(window[0], temp);
        *slot = new;
        window = [
            window[1], window[2], window[3], window[4], window[5], window[6], window[7], new,
        ];
        index = index.wrapping_add(1);
    }
    expanded
}
