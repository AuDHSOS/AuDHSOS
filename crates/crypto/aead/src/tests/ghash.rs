// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! GHASH on its own. The value the second published GCM case implies is
//! the tag of that case exclusive-ored with the encryption of its first
//! counter block, both of which the case documents.

use crate::ghash::{BLOCK_LEN, GHash};
use crate::tests::{hex, unhex};

/// The hash key of the published cases with an all-zero key.
fn hash_key() -> [u8; BLOCK_LEN] {
    let mut key = [0u8; BLOCK_LEN];
    for (slot, byte) in key
        .iter_mut()
        .zip(unhex("66e94bd4ef8a2c3b884cfa59ca342b2e"))
    {
        *slot = byte;
    }
    key
}

#[test]
fn the_second_published_case_hashes_to_what_its_tag_implies() {
    let mut hash = GHash::new(&hash_key());
    hash.update(&unhex("0388dace60b6a392f328c2b971b2fe78"));
    hash.update(&unhex("00000000000000000000000000000080"));
    assert_eq!(hex(&hash.finish()), "f38cbb1ad69223dcc3457ae5b6b0f885");
}

#[test]
fn the_empty_message_hashes_to_zero() {
    assert_eq!(
        hex(&GHash::new(&hash_key()).finish()),
        "00000000000000000000000000000000"
    );
}

#[test]
fn a_partial_block_is_padded_with_zeros() {
    let mut partial = GHash::new(&hash_key());
    partial.update(&[1, 2, 3, 4, 5]);

    let mut padded = GHash::new(&hash_key());
    let mut block = [0u8; BLOCK_LEN];
    for (slot, byte) in block.iter_mut().zip([1u8, 2, 3, 4, 5]) {
        *slot = byte;
    }
    padded.update(&block);

    assert_eq!(partial.finish(), padded.finish());
}

#[test]
fn any_split_of_the_message_hashes_the_same() {
    let message: Vec<u8> = (0u8..=200).collect();
    let mut whole = GHash::new(&hash_key());
    whole.update(&message);
    let expected = whole.finish();

    for split in 0..=message.len() {
        let (head, tail) = message.split_at(split);
        let mut pieces = GHash::new(&hash_key());
        pieces.update(head);
        pieces.update(tail);
        assert_eq!(pieces.finish(), expected, "split at {split}");
    }
}

#[test]
fn clearing_a_computation_drops_the_hash_key_and_the_accumulator() {
    let mut hash = GHash::new(&hash_key());
    hash.update(&[0xAAu8; 24]);
    hash.clear();
    assert_eq!(hex(&hash.finish()), hex(&[0u8; BLOCK_LEN]));
}

#[test]
fn a_cleared_computation_keeps_no_partial_block() {
    let mut hash = GHash::new(&[0x99u8; BLOCK_LEN]);
    hash.update(&[0x01u8; 5]);
    hash.clear();
    let empty = GHash::new(&[0u8; BLOCK_LEN]);
    assert_eq!(hex(&hash.finish()), hex(&empty.finish()));
}

/// The reduction polynomial of GF(2^128) in the bit order of the standard.
const REDUCTION: u128 = 0xE100_0000_0000_0000_0000_0000_0000_0000;

/// Multiplication in GF(2^128) one bit at a time, the algorithm NIST SP
/// 800-38D, section 6.3, writes out. The product this returns is what the
/// carry-less form in the module under test has to agree with.
fn reference_multiply(x: u128, y: u128) -> u128 {
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

/// The hash of one block under one key, by the reference multiplication.
fn reference_hash(key: &[u8; BLOCK_LEN], block: &[u8; BLOCK_LEN]) -> [u8; BLOCK_LEN] {
    let product = reference_multiply(u128::from_be_bytes(*block), u128::from_be_bytes(*key));
    product.to_be_bytes()
}

/// A value of the sequence the test drives the two implementations with:
/// a multiplication in the field itself, so that the bits spread.
fn next(state: u128) -> u128 {
    reference_multiply(state ^ 0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C834, 2)
}

/// Regression for issue #46: the multiplication ran 128 steps over `u128`
/// and now runs three 64-bit carry-less products with one reduction. The
/// two forms have to give the same product for every pair, which the
/// published vectors alone do not show.
#[test]
fn the_carryless_product_agrees_with_the_bit_at_a_time_form() {
    let mut key_state = 1u128;
    let mut block_state = 0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFFu128;
    for round in 0..256 {
        let key = key_state.to_be_bytes();
        let block = block_state.to_be_bytes();

        let mut hash = GHash::new(&key);
        hash.update(&block);
        assert_eq!(hash.finish(), reference_hash(&key, &block), "round {round}");

        key_state = next(key_state);
        block_state = next(block_state);
    }
}

/// Regression for issue #46: the edge operands a shift-and-reduce form and
/// a carry-less form disagree on first, all of them pinned to the bit-at-a-
/// time reference.
#[test]
fn the_edge_operands_agree_with_the_bit_at_a_time_form() {
    let operands: [u128; 6] = [
        0,
        1,
        1u128 << 127,
        u128::MAX,
        0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10,
        0xFFFF_FFFF_FFFF_FFFF_0000_0000_0000_0000,
    ];
    for key in operands {
        for block in operands {
            let key = key.to_be_bytes();
            let block = block.to_be_bytes();
            let mut hash = GHash::new(&key);
            hash.update(&block);
            assert_eq!(hash.finish(), reference_hash(&key, &block));
        }
    }
}
