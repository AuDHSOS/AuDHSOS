// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The digest `md5` of `testfixture` answers, which is RFC 1321.
//!
//! The suite's `cksum` holds one state of a database against another and
//! a `VACUUM` is right where the two digests agree, so the digest is a
//! checksum of what the engine answered and no part of the engine.
//!
//! Section 3 of `docs/rfc/rfc1321.txt` gives the five steps, section 3.4
//! the table of 64 sine constants and the shift amounts, and section A.5
//! the seven test digests the tests below hold this against.

/// The shift amount of each of the 64 operations, four rounds of four
/// amounts each, which is `S11` to `S44` of section 3.4.
const SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, //
    5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, //
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, //
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// `T[i]` of section 3.4, which is 4294967296 times the absolute value of
/// the sine of `i` radians, the fraction taken.
const SINES: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

/// The digest of `bytes` as the 32 lowercase hexadecimal digits `md5`
/// answers.
///
/// One block of 64 bytes costs 64 operations, so the digest costs O(n) in
/// the bytes.
pub(crate) fn digest(bytes: &[u8]) -> String {
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    // Step 1 and step 2 of section 3: a one bit, then zero bits until 56
    // bytes of the block are taken, then the length in bits as two
    // 32-bit words with the low word first.
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    let bits = u64::try_from(bytes.len()).unwrap_or(0).wrapping_mul(8);
    padded.extend_from_slice(&bits.to_le_bytes());
    for block in padded.chunks(64) {
        run(&mut state, block);
    }
    let mut out = String::with_capacity(32);
    for word in state {
        for byte in word.to_le_bytes() {
            out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            out.push(char::from_digit(u32::from(byte & 0xf), 16).unwrap_or('0'));
        }
    }
    out
}

/// One block of 64 bytes run over the four words, which is step 4 of
/// section 3.
fn run(state: &mut [u32; 4], block: &[u8]) {
    let mut words = [0_u32; 16];
    for (at, slot) in words.iter_mut().enumerate() {
        let mut held = [0_u8; 4];
        for (byte, taken) in held.iter_mut().zip(block.iter().skip(at.saturating_mul(4))) {
            *byte = *taken;
        }
        *slot = u32::from_le_bytes(held);
    }
    let [mut a, mut b, mut c, mut d] = *state;
    for step in 0_usize..64 {
        // The four rounds of section 3: F, G, H and I, each over a
        // different order of the sixteen words.
        let (mixed, at) = match step / 16 {
            0 => ((b & c) | (!b & d), step),
            1 => (
                (d & b) | (!d & c),
                (step.wrapping_mul(5).wrapping_add(1)) % 16,
            ),
            2 => (b ^ c ^ d, (step.wrapping_mul(3).wrapping_add(5)) % 16),
            _ => (c ^ (b | !d), step.wrapping_mul(7) % 16),
        };
        let word = words.get(at).copied().unwrap_or(0);
        let sine = SINES.get(step).copied().unwrap_or(0);
        let shift = SHIFTS.get(step).copied().unwrap_or(0);
        let held = a
            .wrapping_add(mixed)
            .wrapping_add(word)
            .wrapping_add(sine)
            .rotate_left(shift);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(held);
    }
    for (slot, held) in state.iter_mut().zip([a, b, c, d]) {
        *slot = slot.wrapping_add(held);
    }
}

#[cfg(test)]
mod tests {
    /// Section A.5 of `docs/rfc/rfc1321.txt`: the seven digests the
    /// document's own driver prints.
    #[test]
    fn the_seven_digests_of_the_document() {
        for (text, digest) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ] {
            assert_eq!(super::digest(text.as_bytes()), digest, "{text}");
        }
    }
}
