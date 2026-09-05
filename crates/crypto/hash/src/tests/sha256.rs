// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! SHA-256 against the vectors of FIPS 180-4 and its appendices.

use test_support::generators::bytes;
use test_support::property::check;

use crate::hash::Hash;
use crate::sha256::Sha256;
use crate::tests::hex;

#[test]
fn the_fips_vectors_hash_as_documented() {
    assert_eq!(
        hex(&Sha256::digest(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(&Sha256::digest(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&Sha256::digest(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        hex(&Sha256::digest(
            b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"
        )),
        "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"
    );
}

#[test]
fn the_million_character_message_hashes_as_documented() {
    let mut state = Sha256::new();
    for _ in 0..1_000 {
        state.update(&[b'a'; 1_000]);
    }
    assert_eq!(
        hex(&state.finish()),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn the_padding_boundaries_hash_correctly() {
    let cases: [(usize, &str); 9] = [
        (
            55,
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
        ),
        (
            56,
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
        ),
        (
            57,
            "f13b2d724659eb3bf47f2dd6af1accc87b81f09f59f2b75e5c0bed6589dfe8c6",
        ),
        (
            63,
            "7d3e74a05d7db15bce4ad9ec0658ea98e3f06eeecf16b4c6fff2da457ddc2f34",
        ),
        (
            64,
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
        ),
        (
            65,
            "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0",
        ),
        (
            119,
            "31eba51c313a5c08226adf18d4a359cfdfd8d2e816b13f4af952f7ea6584dcfb",
        ),
        (
            120,
            "2f3d335432c70b580af0e8e1b3674a7c020d683aa5f73aaaedfdc55af904c21c",
        ),
        (
            128,
            "6836cf13bac400e9105071cd6af47084dfacad4e5e302c94bfed24e013afb73e",
        ),
    ];
    for (length, expected) in cases {
        let message = vec![b'a'; length];
        assert_eq!(hex(&Sha256::digest(&message)), expected, "length {length}");
    }
}

#[test]
fn the_trait_agrees_with_the_inherent_functions() {
    assert_eq!(<Sha256 as Hash>::digest(b"abc"), Sha256::digest(b"abc"));
    let mut state = <Sha256 as Hash>::new();
    Hash::update(&mut state, b"abc");
    assert_eq!(Hash::finish(state), Sha256::digest(b"abc"));
    assert_eq!(Sha256::default().finish(), Sha256::digest(b""));
    assert_eq!(<Sha256 as Hash>::BLOCK_LEN, 64);
    assert_eq!(<Sha256 as Hash>::OUTPUT_LEN, 32);
    assert_eq!(<Sha256 as Hash>::ZERO_BLOCK, [0u8; 64]);
}

#[test]
fn a_clone_continues_the_same_message() {
    let mut state = Sha256::new();
    state.update(b"abcdef");
    let clone = state.clone();
    state.update(b"ghi");
    assert_eq!(clone.finish(), Sha256::digest(b"abcdef"));
    assert_eq!(state.finish(), Sha256::digest(b"abcdefghi"));
}

#[test]
fn property_any_split_of_a_message_gives_the_same_digest() {
    check("sha256_chunked", &bytes(0..=200), |message| {
        let expected = Sha256::digest(message);
        for split in 0..=message.len() {
            let (head, tail) = message.split_at(split);
            let mut state = Sha256::new();
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
fn property_byte_at_a_time_matches_the_one_shot_digest() {
    check("sha256_byte_at_a_time", &bytes(0..=140), |message| {
        let mut state = Sha256::new();
        for byte in message {
            state.update(&[*byte]);
        }
        if state.finish() == Sha256::digest(message) {
            Ok(())
        } else {
            Err("byte-wise hashing differs".to_owned())
        }
    });
}
