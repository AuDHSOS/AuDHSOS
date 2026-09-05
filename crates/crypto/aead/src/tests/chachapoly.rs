// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ChaCha20-Poly1305` against the vector of RFC 8439, section 2.8.2.

use test_support::generators::{bytes, vec};
use test_support::property::check;

use crate::aead::{Aead, TAG_LEN};
use crate::chachapoly::ChaCha20Poly1305;
use crate::error::AeadError;
use crate::tests::{hex, unhex};

/// The plaintext of RFC 8439, section 2.8.2.
const MESSAGE: &[u8] = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

/// The key of the vector: the bytes 0x80 to 0x9f.
fn key() -> [u8; 32] {
    let mut key = [0u8; 32];
    for (slot, value) in key.iter_mut().zip(0x80u8..) {
        *slot = value;
    }
    key
}

/// The nonce of the vector.
fn nonce() -> [u8; 12] {
    [
        0x07, 0, 0, 0, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47,
    ]
}

/// The associated data of the vector.
fn aad() -> Vec<u8> {
    unhex("50515253c0c1c2c3c4c5c6c7")
}

#[test]
fn the_rfc_vector_seals_as_documented() {
    let cipher = ChaCha20Poly1305::from_key(&key());
    let mut buffer = MESSAGE.to_vec();
    let tag = cipher
        .seal(&nonce(), &aad(), &mut buffer)
        .expect("the vector is well formed");
    assert_eq!(
        hex(&buffer),
        "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b6116"
    );
    assert_eq!(hex(&tag), "1ae10b594f09e26a7e902ecbd0600691");
}

#[test]
fn what_was_sealed_opens_again() {
    let cipher = ChaCha20Poly1305::from_key(&key());
    let mut buffer = MESSAGE.to_vec();
    let tag = cipher
        .seal(&nonce(), &aad(), &mut buffer)
        .expect("the vector is well formed");
    assert_eq!(cipher.open(&nonce(), &aad(), &mut buffer, &tag), Ok(()));
    assert_eq!(buffer, MESSAGE);
}

#[test]
fn an_empty_message_and_empty_associated_data_are_accepted() {
    let cipher = ChaCha20Poly1305::from_key(&key());
    let mut buffer: Vec<u8> = Vec::new();
    let tag = cipher
        .seal(&nonce(), b"", &mut buffer)
        .expect("an empty message is well formed");
    assert!(buffer.is_empty());
    assert_eq!(hex(&tag), "a0784d7a4716f3feb4f64e7f4b39bf04");
    assert_eq!(cipher.open(&nonce(), b"", &mut buffer, &tag), Ok(()));
}

#[test]
fn a_message_without_associated_data_is_accepted() {
    let cipher = ChaCha20Poly1305::from_key(&key());
    let mut buffer = MESSAGE.to_vec();
    let tag = cipher
        .seal(&nonce(), b"", &mut buffer)
        .expect("the message is well formed");
    assert_eq!(hex(&tag), "6a23a4681fd59456aea1d29f82477216");
}

#[test]
fn every_change_to_the_tag_is_caught_and_the_buffer_is_cleared() {
    let cipher = ChaCha20Poly1305::from_key(&key());
    for position in 0..TAG_LEN {
        let mut buffer = MESSAGE.to_vec();
        let mut tag = cipher
            .seal(&nonce(), &aad(), &mut buffer)
            .expect("the vector is well formed");
        if let Some(byte) = tag.get_mut(position) {
            *byte ^= 0x01;
        }
        assert_eq!(
            cipher.open(&nonce(), &aad(), &mut buffer, &tag),
            Err(AeadError::BadTag),
            "flipped tag byte {position}"
        );
        assert!(
            buffer.iter().all(|byte| *byte == 0),
            "a failed open leaves nothing usable in the buffer"
        );
    }
}

#[test]
fn a_change_to_the_ciphertext_the_data_or_the_nonce_is_caught() {
    let cipher = ChaCha20Poly1305::from_key(&key());

    let mut buffer = MESSAGE.to_vec();
    let tag = cipher
        .seal(&nonce(), &aad(), &mut buffer)
        .expect("the vector is well formed");

    for position in 0..buffer.len() {
        let mut damaged = buffer.clone();
        if let Some(byte) = damaged.get_mut(position) {
            *byte ^= 0x80;
        }
        assert_eq!(
            cipher.open(&nonce(), &aad(), &mut damaged, &tag),
            Err(AeadError::BadTag),
            "flipped ciphertext byte {position}"
        );
    }

    let mut other_data = buffer.clone();
    let mut changed = aad();
    if let Some(byte) = changed.first_mut() {
        *byte ^= 0x01;
    }
    assert_eq!(
        cipher.open(&nonce(), &changed, &mut other_data, &tag),
        Err(AeadError::BadTag)
    );

    let mut other_nonce = buffer.clone();
    let mut moved = nonce();
    moved[11] ^= 0x01;
    assert_eq!(
        cipher.open(&moved, &aad(), &mut other_nonce, &tag),
        Err(AeadError::BadTag)
    );

    let mut other_key = buffer.clone();
    let stranger = ChaCha20Poly1305::from_key(&[0x11u8; 32]);
    assert_eq!(
        stranger.open(&nonce(), &aad(), &mut other_key, &tag),
        Err(AeadError::BadTag)
    );
}

#[test]
fn a_key_or_a_nonce_of_the_wrong_length_is_refused() {
    assert_eq!(
        ChaCha20Poly1305::new(&[0u8; 31]).err(),
        Some(AeadError::KeyLength)
    );
    assert_eq!(
        ChaCha20Poly1305::new(&[0u8; 33]).err(),
        Some(AeadError::KeyLength)
    );
    let cipher = ChaCha20Poly1305::new(&[0u8; 32]).expect("thirty-two bytes are a key");
    let mut buffer = [0u8; 4];
    assert_eq!(
        cipher.seal(&[0u8; 11], b"", &mut buffer),
        Err(AeadError::NonceLength)
    );
    assert_eq!(
        cipher.open(&[0u8; 13], b"", &mut buffer, &[0u8; TAG_LEN]),
        Err(AeadError::NonceLength)
    );
}

#[test]
fn the_trait_reports_the_lengths_of_the_algorithm() {
    assert_eq!(<ChaCha20Poly1305 as Aead>::KEY_LEN, 32);
    assert_eq!(<ChaCha20Poly1305 as Aead>::NONCE_LEN, 12);
    assert_eq!(TAG_LEN, 16);
}

#[test]
fn the_errors_render_a_message() {
    for error in [
        AeadError::KeyLength,
        AeadError::NonceLength,
        AeadError::MessageTooLong,
        AeadError::BadTag,
    ] {
        assert!(!format!("{error}").is_empty());
        assert!(!format!("{error:?}").is_empty());
    }
}

#[test]
fn property_what_is_sealed_opens_and_nothing_else_does() {
    let cipher = ChaCha20Poly1305::from_key(&key());
    check(
        "chachapoly_round_trip",
        &vec(bytes(0..=60), 2..=2),
        |parts| {
            let (Some(message), Some(data)) = (parts.first(), parts.get(1)) else {
                return Err("the generator produced fewer than two values".to_owned());
            };
            let mut buffer = message.clone();
            let tag = cipher
                .seal(&nonce(), data, &mut buffer)
                .map_err(|error| format!("seal: {error}"))?;
            if !message.is_empty() && buffer == *message {
                return Err("the ciphertext equals the plaintext".to_owned());
            }
            cipher
                .open(&nonce(), data, &mut buffer, &tag)
                .map_err(|error| format!("open: {error}"))?;
            if buffer != *message {
                return Err("the message did not come back".to_owned());
            }

            let mut again = message.clone();
            let tag = cipher
                .seal(&nonce(), data, &mut again)
                .map_err(|error| format!("seal: {error}"))?;
            let mut wrong = data.clone();
            wrong.push(0);
            if cipher.open(&nonce(), &wrong, &mut again, &tag).is_ok() {
                return Err("longer associated data still opened".to_owned());
            }
            Ok(())
        },
    );
}
