// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `Poly1305` against the vector of RFC 8439, section 2.5.2.

use crypto_ct::Choice;
use test_support::generators::bytes;
use test_support::property::check;

use crate::poly1305::{BLOCK_LEN, Poly1305};
use crate::tests::{hex, unhex};

/// The key of RFC 8439, section 2.5.2.
fn key() -> [u8; 32] {
    let mut key = [0u8; 32];
    for (slot, byte) in key.iter_mut().zip(unhex(
        "85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b",
    )) {
        *slot = byte;
    }
    key
}

#[test]
fn the_rfc_vector_authenticates_as_documented() {
    let tag = Poly1305::tag_of(&key(), b"Cryptographic Forum Research Group");
    assert_eq!(hex(&tag), "a8061dc1305136c6c22b8baf0c0127a9");
}

#[test]
fn the_empty_message_gives_the_addend_of_the_key() {
    assert_eq!(
        hex(&Poly1305::tag_of(&key(), b"")),
        "0103808afb0db2fd4abff6af4149f51b"
    );
}

#[test]
fn a_message_of_exactly_one_block_needs_no_padding() {
    let message: Vec<u8> = (0u8..16).collect();
    assert_eq!(
        hex(&Poly1305::tag_of(&key(), &message)),
        "a18a0de2ba299128303a398e28bde4f0"
    );
    assert_eq!(message.len(), BLOCK_LEN);
}

#[test]
fn a_message_one_byte_past_a_block_is_padded() {
    let message: Vec<u8> = (0u8..17).collect();
    assert_eq!(
        hex(&Poly1305::tag_of(&key(), &message)),
        "37477d65160c3ca0466aac5780785ef5"
    );
}

#[test]
fn verification_accepts_the_right_tag_and_rejects_every_other() {
    let message = b"Cryptographic Forum Research Group";
    let tag = Poly1305::tag_of(&key(), message);
    assert_eq!(Poly1305::verify(&key(), message, &tag), Choice::YES);

    for position in 0..tag.len() {
        let mut damaged = tag;
        if let Some(byte) = damaged.get_mut(position) {
            *byte ^= 0x01;
        }
        assert_eq!(
            Poly1305::verify(&key(), message, &damaged),
            Choice::NO,
            "flipped byte {position}"
        );
    }

    assert_eq!(
        Poly1305::verify(&key(), b"another message", &tag),
        Choice::NO
    );
    assert_eq!(Poly1305::verify(&key(), message, &tag[..8]), Choice::NO);
}

#[test]
fn property_any_split_of_a_message_gives_the_same_tag() {
    check("poly1305_chunked", &bytes(0..=200), |message| {
        let expected = Poly1305::tag_of(&key(), message);
        for split in 0..=message.len() {
            let (head, tail) = message.split_at(split);
            let mut state = Poly1305::new(&key());
            state.update(head);
            state.update(tail);
            if state.finish() != expected {
                return Err(format!("split at {split} differs"));
            }
        }
        Ok(())
    });
}

#[test]
fn property_byte_at_a_time_matches_the_one_shot_tag() {
    check("poly1305_byte_at_a_time", &bytes(0..=140), |message| {
        let mut state = Poly1305::new(&key());
        for byte in message {
            state.update(&[*byte]);
        }
        if state.finish() == Poly1305::tag_of(&key(), message) {
            Ok(())
        } else {
            Err("byte-wise authentication differs".to_owned())
        }
    });
}
