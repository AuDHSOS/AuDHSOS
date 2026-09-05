// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ChaCha20` against the vectors of RFC 8439, sections 2.3.2 and 2.4.2.

use test_support::generators::bytes;
use test_support::property::check;

use crate::chacha20::{BLOCK_LEN, ChaCha20};
use crate::error::AeadError;
use crate::tests::hex;

/// The message of RFC 8439, section 2.4.2.
const MESSAGE: &[u8] = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

/// The key of both vectors: the bytes zero to thirty-one.
fn key() -> [u8; 32] {
    let mut key = [0u8; 32];
    for (slot, value) in key.iter_mut().zip(0u8..) {
        *slot = value;
    }
    key
}

#[test]
fn the_block_function_matches_the_rfc_vector() {
    let nonce = [0, 0, 0, 9, 0, 0, 0, 0x4a, 0, 0, 0, 0];
    let block = ChaCha20::new(&key()).block(&nonce, 1);
    assert_eq!(
        hex(&block),
        "10f1e7e4d13b5915500fdd1fa32071c4c7d1f4c733c068030422aa9ac3d46c4ed2826446079faa0914c2d705d98b02a2b5129cd1de164eb9cbd083e8a2503c4e"
    );
}

#[test]
fn the_keystream_encrypts_the_rfc_message() {
    let nonce = [0, 0, 0, 0, 0, 0, 0, 0x4a, 0, 0, 0, 0];
    let cipher = ChaCha20::new(&key());
    let mut buffer = MESSAGE.to_vec();
    cipher
        .apply_keystream(&nonce, 1, &mut buffer)
        .expect("the message is far below the counter limit");
    assert_eq!(
        hex(&buffer),
        "6e2e359a2568f98041ba0728dd0d6981e97e7aec1d4360c20a27afccfd9fae0bf91b65c5524733ab8f593dabcd62b3571639d624e65152ab8f530c359f0861d807ca0dbf500d6a6156a38e088a22b65e52bc514d16ccf806818ce91ab77937365af90bbf74a35be6b40b8eedf2785e42874d"
    );

    cipher
        .apply_keystream(&nonce, 1, &mut buffer)
        .expect("the message is far below the counter limit");
    assert_eq!(buffer, MESSAGE, "the stream is its own inverse");
}

#[test]
fn an_empty_message_is_left_alone() {
    let mut empty: [u8; 0] = [];
    assert_eq!(
        ChaCha20::new(&key()).apply_keystream(&[0u8; 12], 0, &mut empty),
        Ok(())
    );
}

#[test]
fn a_message_across_blocks_continues_the_counter() {
    let nonce = [7u8; 12];
    let cipher = ChaCha20::new(&key());
    let mut whole = vec![0u8; BLOCK_LEN * 2 + 5];
    cipher
        .apply_keystream(&nonce, 3, &mut whole)
        .expect("three blocks are far below the counter limit");

    let mut pieces = vec![0u8; BLOCK_LEN * 2 + 5];
    let (head, tail) = pieces.split_at_mut(BLOCK_LEN);
    cipher
        .apply_keystream(&nonce, 3, head)
        .expect("one block is far below the counter limit");
    cipher
        .apply_keystream(&nonce, 4, tail)
        .expect("two blocks are far below the counter limit");

    assert_eq!(whole, pieces);
}

#[test]
fn the_counter_limit_is_refused_before_anything_is_written() {
    let cipher = ChaCha20::new(&key());
    let nonce = [0u8; 12];

    let mut last_block = vec![0u8; BLOCK_LEN];
    assert_eq!(
        cipher.apply_keystream(&nonce, u32::MAX, &mut last_block),
        Ok(()),
        "the very last block is still inside the range"
    );

    let mut beyond = vec![0u8; BLOCK_LEN + 1];
    assert_eq!(
        cipher.apply_keystream(&nonce, u32::MAX, &mut beyond),
        Err(AeadError::MessageTooLong)
    );
    assert!(
        beyond.iter().all(|byte| *byte == 0),
        "a refused message stays untouched"
    );
}

#[test]
fn property_applying_the_keystream_twice_restores_the_message() {
    let cipher = ChaCha20::new(&key());
    check("chacha20_involution", &bytes(0..=300), |message| {
        let mut buffer = message.clone();
        cipher
            .apply_keystream(&[9u8; 12], 1, &mut buffer)
            .map_err(|error| format!("{error}"))?;
        if buffer == *message && !message.is_empty() {
            return Err("the keystream left the message unchanged".to_owned());
        }
        cipher
            .apply_keystream(&[9u8; 12], 1, &mut buffer)
            .map_err(|error| format!("{error}"))?;
        if buffer == *message {
            Ok(())
        } else {
            Err("the message did not come back".to_owned())
        }
    });
}
