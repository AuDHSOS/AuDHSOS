// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The group parameters against the document they were taken from.
//!
//! The prime below is the hexadecimal value RFC 3526, section 3, prints,
//! transcribed row by row exactly as that section sets it: eleven rows,
//! ten of six groups of four bytes and a last one of four. It is written
//! out a second time here so that the array in the product is checked
//! against a transcription rather than against itself.

use crate::group::{
    GROUP14_GENERATOR, GROUP14_PRIME, GROUP14_PRIME_BYTES, GROUP14_SECRET_BYTES, group14,
};
use crate::tests::{hex, unhex};

/// The prime of the 2048-bit MODP group, RFC 3526, section 3.
const RFC3526_SECTION3_PRIME: &str = "\
    FFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1 \
    29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD \
    EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245 \
    E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED \
    EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE45B3D \
    C2007CB8 A163BF05 98DA4836 1C55D39A 69163FA8 FD24CF5F \
    83655D23 DCA3AD96 1C62F356 208552BB 9ED52907 7096966D \
    670C354E 4ABC9804 F1746C08 CA18217C 32905E46 2E36CE3B \
    E39E772C 180E8603 9B2783A2 EC07A28F B5C55DF0 6F4C52C9 \
    DE2BCBF6 95581718 3995497C EA956AE5 15D22618 98FA0510 \
    15728E5A 8AACAA68 FFFFFFFF FFFFFFFF";

#[test]
fn the_prime_is_the_value_the_document_prints() {
    let expected = unhex(RFC3526_SECTION3_PRIME);
    assert_eq!(expected.len(), GROUP14_PRIME_BYTES);
    assert_eq!(hex(&GROUP14_PRIME), hex(&expected));
}

/// The closed form the document gives above the hexadecimal is
/// `2^2048 - 2^1984 - 1 + 2^64 * { [2^1918 pi] + 124476 }`. The two ends
/// of it are checkable without pi: the leading `2^2048 - 2^1984` is
/// sixty-four bits of ones at the top, and the trailing `- 1` over a
/// multiple of `2^64` is sixty-four bits of ones at the bottom.
#[test]
fn the_prime_has_the_width_and_the_two_ends_the_closed_form_gives_it() {
    assert_eq!(GROUP14_PRIME_BYTES, 256);
    assert_eq!(GROUP14_PRIME.len() * 8, 2048);
    assert_eq!(GROUP14_PRIME.get(..8), Some(&[0xFFu8; 8][..]));
    assert_eq!(GROUP14_PRIME.get(248..), Some(&[0xFFu8; 8][..]));
}

#[test]
fn the_generator_is_the_two_the_document_states() {
    assert_eq!(GROUP14_GENERATOR, 2);
}

#[test]
fn the_recommended_exponent_is_two_hundred_and_fifty_six_bits() {
    assert_eq!(GROUP14_SECRET_BYTES * 8, 256);
}

#[test]
fn the_group_of_the_document_is_built_and_is_two_thousand_and_forty_eight_bits_wide() {
    let group = group14().expect("the constants of the document form a group");
    assert_eq!(group.public_len(), GROUP14_PRIME_BYTES);
}
