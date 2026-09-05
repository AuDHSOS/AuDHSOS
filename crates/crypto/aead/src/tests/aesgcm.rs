// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! AES-GCM against the test cases published with the specification of the
//! mode, which NIST SP 800-38D adopted: cases one to four for AES-128 and
//! thirteen to sixteen for AES-256.

use test_support::generators::{bytes, vec};
use test_support::property::check;

use crate::aead::{Aead, TAG_LEN};
use crate::aes::Aes;
use crate::aesgcm::{Aes128Gcm, Aes256Gcm};
use crate::error::AeadError;
use crate::tests::{hex, unhex};

/// One published test case.
struct Case {
    /// The number the specification gives the case.
    name: &'static str,
    /// The key.
    key: &'static str,
    /// The nonce.
    nonce: &'static str,
    /// The plaintext.
    plaintext: &'static str,
    /// The associated data.
    aad: &'static str,
    /// The expected ciphertext.
    ciphertext: &'static str,
    /// The expected tag.
    tag: &'static str,
}

/// The cases with a 128-bit key.
const SHORT_KEY: [Case; 4] = [
    Case {
        name: "1",
        key: "00000000000000000000000000000000",
        nonce: "000000000000000000000000",
        plaintext: "",
        aad: "",
        ciphertext: "",
        tag: "58e2fccefa7e3061367f1d57a4e7455a",
    },
    Case {
        name: "2",
        key: "00000000000000000000000000000000",
        nonce: "000000000000000000000000",
        plaintext: "00000000000000000000000000000000",
        aad: "",
        ciphertext: "0388dace60b6a392f328c2b971b2fe78",
        tag: "ab6e47d42cec13bdf53a67b21257bddf",
    },
    Case {
        name: "3",
        key: "feffe9928665731c6d6a8f9467308308",
        nonce: "cafebabefacedbaddecaf888",
        plaintext: "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255",
        aad: "",
        ciphertext: "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091473f5985",
        tag: "4d5c2af327cd64a62cf35abd2ba6fab4",
    },
    Case {
        name: "4",
        key: "feffe9928665731c6d6a8f9467308308",
        nonce: "cafebabefacedbaddecaf888",
        plaintext: "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
        aad: "feedfacedeadbeeffeedfacedeadbeefabaddad2",
        ciphertext: "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091",
        tag: "5bc94fbc3221a5db94fae95ae7121a47",
    },
];

/// The cases with a 256-bit key.
const LONG_KEY: [Case; 4] = [
    Case {
        name: "13",
        key: "0000000000000000000000000000000000000000000000000000000000000000",
        nonce: "000000000000000000000000",
        plaintext: "",
        aad: "",
        ciphertext: "",
        tag: "530f8afbc74536b9a963b4f1c4cb738b",
    },
    Case {
        name: "14",
        key: "0000000000000000000000000000000000000000000000000000000000000000",
        nonce: "000000000000000000000000",
        plaintext: "00000000000000000000000000000000",
        aad: "",
        ciphertext: "cea7403d4d606b6e074ec5d3baf39d18",
        tag: "d0d1c8a799996bf0265b98b5d48ab919",
    },
    Case {
        name: "15",
        key: "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308",
        nonce: "cafebabefacedbaddecaf888",
        plaintext: "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255",
        aad: "",
        ciphertext: "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662898015ad",
        tag: "b094dac5d93471bdec1a502270e3cc6c",
    },
    Case {
        name: "16",
        key: "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308",
        nonce: "cafebabefacedbaddecaf888",
        plaintext: "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
        aad: "feedfacedeadbeeffeedfacedeadbeefabaddad2",
        ciphertext: "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662",
        tag: "76fc6ece0f4e1768cddf8853bb2d551b",
    },
];

#[test]
fn the_hash_key_of_the_published_cases_is_the_documented_one() {
    let mut zero = [0u8; 16];
    Aes::new_128(&[0u8; 16]).encrypt_block(&mut zero);
    assert_eq!(hex(&zero), "66e94bd4ef8a2c3b884cfa59ca342b2e");
}

#[test]
fn the_published_cases_seal_and_open_as_documented() {
    for case in &SHORT_KEY {
        let mut key = [0u8; 16];
        for (slot, byte) in key.iter_mut().zip(unhex(case.key)) {
            *slot = byte;
        }
        run(&Aes128Gcm::from_key(&key), case);
    }
    for case in &LONG_KEY {
        let mut key = [0u8; 32];
        for (slot, byte) in key.iter_mut().zip(unhex(case.key)) {
            *slot = byte;
        }
        run(&Aes256Gcm::from_key(&key), case);
    }
}

/// Seals the case, compares both halves, and opens it again.
fn run<A: Aead>(cipher: &A, case: &Case) {
    let mut buffer = unhex(case.plaintext);
    let aad = unhex(case.aad);
    let nonce = unhex(case.nonce);
    let tag = cipher
        .seal(&nonce, &aad, &mut buffer)
        .expect("the published case is well formed");
    assert_eq!(
        hex(&buffer),
        case.ciphertext,
        "case {} ciphertext",
        case.name
    );
    assert_eq!(hex(&tag), case.tag, "case {} tag", case.name);

    assert_eq!(
        cipher.open(&nonce, &aad, &mut buffer, &tag),
        Ok(()),
        "case {} opens",
        case.name
    );
    assert_eq!(hex(&buffer), case.plaintext, "case {} plaintext", case.name);
}

#[test]
fn lengths_around_the_four_block_group_seal_as_expected() {
    let mut key = [0u8; 16];
    for (slot, byte) in key.iter_mut().zip(0u8..) {
        *slot = byte;
    }
    let nonce: Vec<u8> = (0u8..12).collect();
    let cipher = Aes128Gcm::from_key(&key);

    let cases: [(usize, &str, &str); 13] = [
        (0, "", "f0e30fe446fc5324c9e3a6292ebd8965"),
        (1, "90", "3ec52d6f7f8e5ab5af0a9921a696ab62"),
        (
            15,
            "9066b6d6793dda60709028da61fd15",
            "62801df67b163e4966ff50138db8709e",
        ),
        (
            16,
            "9066b6d6793dda60709028da61fd1564",
            "71f80a83d8769623999adbf66bda98ed",
        ),
        (
            17,
            "9066b6d6793dda60709028da61fd1564c0",
            "ff330eaa129c488886a675742dd619e8",
        ),
        (
            31,
            "9066b6d6793dda60709028da61fd1564c05c9b6edc7b60524d9348edd38a77",
            "009611122fc81ae2f0319e2b0ece5385",
        ),
        (
            63,
            "9066b6d6793dda60709028da61fd1564c05c9b6edc7b60524d9348edd38a77b0664e2a0f0b86baf75bd620d7f7b792de9fc575986bf14bffa5dceb720de2a3",
            "ab5f75669e31c284c2e1b382dcbeefd8",
        ),
        (
            64,
            "9066b6d6793dda60709028da61fd1564c05c9b6edc7b60524d9348edd38a77b0664e2a0f0b86baf75bd620d7f7b792de9fc575986bf14bffa5dceb720de2a358",
            "32ca7268485127c3b112f29023639a4e",
        ),
        (
            65,
            "9066b6d6793dda60709028da61fd1564c05c9b6edc7b60524d9348edd38a77b0664e2a0f0b86baf75bd620d7f7b792de9fc575986bf14bffa5dceb720de2a35859",
            "f23ee360860afcf1bb0c3413275a200d",
        ),
        (
            127,
            "9066b6d6793dda60709028da61fd1564c05c9b6edc7b60524d9348edd38a77b0664e2a0f0b86baf75bd620d7f7b792de9fc575986bf14bffa5dceb720de2a35859c4249680aa27ead832784f6b242d775b96c42346cac0a69d8fbe573bc71f94c38ffaaca9cd12a07bbd972f0417d317c8c99ac88c7b37bd6eeec712ab8489",
            "410933a47f9865a56a170dda382f85ad",
        ),
        (
            128,
            "9066b6d6793dda60709028da61fd1564c05c9b6edc7b60524d9348edd38a77b0664e2a0f0b86baf75bd620d7f7b792de9fc575986bf14bffa5dceb720de2a35859c4249680aa27ead832784f6b242d775b96c42346cac0a69d8fbe573bc71f94c38ffaaca9cd12a07bbd972f0417d317c8c99ac88c7b37bd6eeec712ab848905",
            "83d3b0f93e26fa11a2fdf7b77b26522c",
        ),
        (
            129,
            "9066b6d6793dda60709028da61fd1564c05c9b6edc7b60524d9348edd38a77b0664e2a0f0b86baf75bd620d7f7b792de9fc575986bf14bffa5dceb720de2a35859c4249680aa27ead832784f6b242d775b96c42346cac0a69d8fbe573bc71f94c38ffaaca9cd12a07bbd972f0417d317c8c99ac88c7b37bd6eeec712ab848905ed",
            "277ab9c68782bc6d7b1330f24a1940ec",
        ),
        (
            200,
            "9066b6d6793dda60709028da61fd1564c05c9b6edc7b60524d9348edd38a77b0664e2a0f0b86baf75bd620d7f7b792de9fc575986bf14bffa5dceb720de2a35859c4249680aa27ead832784f6b242d775b96c42346cac0a69d8fbe573bc71f94c38ffaaca9cd12a07bbd972f0417d317c8c99ac88c7b37bd6eeec712ab848905ed3e115b20f18f2c4f6a4d8588b7c2826057723a9582f4e2078cabf812db696ef48c09a315e3825e065c9b99381733cd06bba6e5f0ff9eb7411de9c29b2bc0af4b1bfd568ec72f05",
            "aba1b49feb1e500a38102c4a3996726e",
        ),
    ];
    for (length, ciphertext, tag) in cases {
        let mut buffer: Vec<u8> = (0..length)
            .map(|index| u8::try_from(index.wrapping_mul(7).wrapping_add(3) & 0xFF).unwrap_or(0))
            .collect();
        let produced = cipher
            .seal(&nonce, b"associated", &mut buffer)
            .expect("the message is well formed");
        assert_eq!(hex(&buffer), ciphertext, "length {length} ciphertext");
        assert_eq!(hex(&produced), tag, "length {length} tag");
    }
}

#[test]
fn every_change_to_the_tag_is_caught_and_the_buffer_is_cleared() {
    let cipher = Aes128Gcm::from_key(&[0x2bu8; 16]);
    let nonce = [0x77u8; 12];
    for position in 0..TAG_LEN {
        let mut buffer = b"a message worth protecting".to_vec();
        let mut tag = cipher
            .seal(&nonce, b"data", &mut buffer)
            .expect("the message is well formed");
        if let Some(byte) = tag.get_mut(position) {
            *byte ^= 0x01;
        }
        assert_eq!(
            cipher.open(&nonce, b"data", &mut buffer, &tag),
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
    let cipher = Aes256Gcm::from_key(&[0x5au8; 32]);
    let nonce = [0x11u8; 12];
    let message = b"a message worth protecting".to_vec();

    let mut sealed = message;
    let tag = cipher
        .seal(&nonce, b"data", &mut sealed)
        .expect("the message is well formed");

    let mut damaged = sealed.clone();
    if let Some(byte) = damaged.first_mut() {
        *byte ^= 0x80;
    }
    assert_eq!(
        cipher.open(&nonce, b"data", &mut damaged, &tag),
        Err(AeadError::BadTag)
    );

    let mut other_data = sealed.clone();
    assert_eq!(
        cipher.open(&nonce, b"date", &mut other_data, &tag),
        Err(AeadError::BadTag)
    );

    let mut other_nonce = sealed.clone();
    assert_eq!(
        cipher.open(&[0x12u8; 12], b"data", &mut other_nonce, &tag),
        Err(AeadError::BadTag)
    );
}

#[test]
fn a_key_or_a_nonce_of_the_wrong_length_is_refused() {
    assert_eq!(Aes128Gcm::new(&[0u8; 15]).err(), Some(AeadError::KeyLength));
    assert_eq!(Aes256Gcm::new(&[0u8; 31]).err(), Some(AeadError::KeyLength));
    let cipher = Aes128Gcm::new(&[0u8; 16]).expect("sixteen bytes are a key");
    let mut buffer = [0u8; 8];
    assert_eq!(
        cipher.seal(&[0u8; 11], b"", &mut buffer),
        Err(AeadError::NonceLength)
    );
    assert_eq!(
        cipher.open(&[0u8; 16], b"", &mut buffer, &[0u8; TAG_LEN]),
        Err(AeadError::NonceLength)
    );
    let long = Aes256Gcm::new(&[0u8; 32]).expect("thirty-two bytes are a key");
    assert_eq!(
        long.seal(&[0u8; 13], b"", &mut buffer),
        Err(AeadError::NonceLength)
    );
    assert_eq!(
        long.open(&[0u8; 13], b"", &mut buffer, &[0u8; TAG_LEN]),
        Err(AeadError::NonceLength)
    );
}

#[test]
fn the_trait_reports_the_lengths_of_the_algorithms() {
    assert_eq!(<Aes128Gcm as Aead>::KEY_LEN, 16);
    assert_eq!(<Aes256Gcm as Aead>::KEY_LEN, 32);
    assert_eq!(<Aes128Gcm as Aead>::NONCE_LEN, 12);
    assert_eq!(<Aes256Gcm as Aead>::NONCE_LEN, 12);
}

#[test]
fn property_what_is_sealed_opens_and_nothing_else_does() {
    let cipher = Aes128Gcm::from_key(&[0x33u8; 16]);
    check("aesgcm_round_trip", &vec(bytes(0..=80), 2..=2), |parts| {
        let (Some(message), Some(data)) = (parts.first(), parts.get(1)) else {
            return Err("the generator produced fewer than two values".to_owned());
        };
        let mut buffer = message.clone();
        let tag = cipher
            .seal(&[4u8; 12], data, &mut buffer)
            .map_err(|error| format!("seal: {error}"))?;
        if !message.is_empty() && buffer == *message {
            return Err("the ciphertext equals the plaintext".to_owned());
        }
        cipher
            .open(&[4u8; 12], data, &mut buffer, &tag)
            .map_err(|error| format!("open: {error}"))?;
        if buffer != *message {
            return Err("the message did not come back".to_owned());
        }

        let mut again = message.clone();
        let tag = cipher
            .seal(&[4u8; 12], data, &mut again)
            .map_err(|error| format!("seal: {error}"))?;
        if cipher.open(&[5u8; 12], data, &mut again, &tag).is_ok() {
            return Err("another nonce still opened".to_owned());
        }
        Ok(())
    });
}
