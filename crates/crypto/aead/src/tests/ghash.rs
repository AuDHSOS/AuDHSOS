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
