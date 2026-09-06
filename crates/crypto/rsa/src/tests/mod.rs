// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.

mod bigint;
mod key;
mod pkcs1;
mod pss;

use crate::key::PublicKey;

/// The modulus of the RSA key of RFC 8448, section 2. It is a thousand
/// and twenty-four bits, below what a certificate may carry (D-79), so it
/// exercises this crate and no chain — which is what it is wanted for.
pub(crate) const RFC8448_MODULUS: &str = "\
    b4bb498f8279303d980836399b36c6988c0c68de55e1bdb826d3901a2461eafd\
    2de49a91d015abbc9a95137ace6c1af19eaa6af98c7ced43120998e187a80ee0\
    ccb0524b1b018c3e0b63264d449a6d38e22a5fda430846748030530ef0461c8c\
    a9d9efbfae8ea6d1d03e2bd193eff0ab9a8002c47428a6d35a8d88d79f7f1e3f";

/// The public exponent of that key, sixty-five thousand five hundred and
/// thirty-seven.
pub(crate) const RFC8448_PUBLIC: &str = "010001";

/// The private exponent of that key, which is what signs in these tests.
pub(crate) const RFC8448_PRIVATE: &str = "\
    04dea705d43a6ea7209dd8072111a83c81e322a59278b33480641eaf7c0a6985\
    b8e31c44f6de62e1b4c2309f6126e77b7c41e923314bbfa3881305dc1217f16c\
    819ce538e922f369828d0e57195d8c8488460207b2faa726bcf708bbd7db7f67\
    9f893492fc2a622e08970aac441ce4e0c3088df25ae679233df8a3bda2ff9941";

/// The key of RFC 8448, section 2, as this crate reads it.
pub(crate) fn rfc8448_key() -> PublicKey {
    PublicKey::new(&unhex(RFC8448_MODULUS), &unhex(RFC8448_PUBLIC))
        .expect("the published key is odd, normalized, and has an odd exponent")
}

/// A modulus of `bits` bits with a fixed body, for the tests that want a
/// width rather than a particular key. It is odd and its top bit is set,
/// which is all this crate asks of one.
pub(crate) fn fixed_modulus(bits: usize) -> Vec<u8> {
    let mut bytes: Vec<u8> = (0..bits / 8)
        .map(|index| u8::try_from(index.wrapping_mul(37).wrapping_add(11) % 251).unwrap_or(1))
        .collect();
    if let Some(first) = bytes.first_mut() {
        *first |= 0x80;
    }
    if let Some(last) = bytes.last_mut() {
        *last |= 0x01;
    }
    bytes
}

/// The bytes of a hexadecimal string.
pub(crate) fn unhex(text: &str) -> Vec<u8> {
    let digits: Vec<u32> = text.chars().filter_map(|c| c.to_digit(16)).collect();
    let (pairs, _) = digits.as_chunks::<2>();
    pairs
        .iter()
        .map(|[high, low]| u8::try_from(high.wrapping_mul(16).wrapping_add(*low)).unwrap_or(0))
        .collect()
}

/// Renders bytes as lower-case hexadecimal.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut rendered = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(rendered, "{byte:02x}");
    }
    rendered
}

/// The modulus of a two-thousand-and-forty-eight-bit key pair, which is
/// what PSS with SHA-512 needs: the encoding wants `hLen + sLen + 2`
/// bytes, a hundred and thirty of them, and the key of RFC 8448 has a
/// hundred and twenty-eight.
///
/// No project code generates an RSA key (D-83). This pair was made once
/// outside this repository with
///
/// ```text
/// openssl genrsa 2048
/// ```
///
/// and its modulus and private exponent are transcribed here. The public
/// exponent is sixty-five thousand five hundred and thirty-seven.
pub(crate) const GENERATED_2048_MODULUS: &str = "\
    c57cf1f2e8a39645e78f5cffa3e61137bd22e8a2187f7ec9875adfe44c8dbc58\
    e9454e3c1dcc1e5a1491d3166bdf5c825bf57b803f1ffa6742d57c3c45d4b33c\
    0c9a95d08d44135d9850c0d7c1d1b1ed89ae9e3e87d0e35da98f8a495057e4f1\
    3b78b5aa3f5cfb23ae96b9c356deb125b98341658b4b03a11f208a3552c6346b\
    1252bcd8b917c7ae6a24cc3a3e83e639cae95ff1edc24f3c52f782f6f9726479\
    62d3b323fc45b659d77ed2ee92557630a039ea596e95eb39a1efd96fc9f7fa93\
    093759b123fc0cde60a6c0a5d3494a64c30bb8e8b873461cea52f6799a5ab232\
    0a2792bc8c8b097e3ca6c7eb891567bd79514c4b225563a617f966a8c1774823";

/// The private exponent of that pair.
pub(crate) const GENERATED_2048_PRIVATE: &str = "\
    0ffe08ed5f7ad56b5b2961c8424266eebc7b1f81c346b3cf67da84f45994ac52\
    a0e212fb4d15392b98df6dcbb2b7fddab434488eb29abc6e0a75fbdf6c6f07b9\
    0e73083c2786027174c2dfb952b4d05a1582e25f7489ed3129cf1367050e7777\
    cde2976de172b16da463d54831c5316d0623839983cc0c9c7dad8033ad0967b7\
    ba4d2b37f12f07123933a999ed466236a385bb140dcb21313cb92a6a9d1a39fe\
    70de4acfd5aaf140085d24c8b9a8a48d48b6ccc5718bff559168dd695a6376ba\
    dc4125dd51290fe1902ee94e60274ef1ec9780b0a9eda9184171b4e224e888e7\
    9a41097f2f3247fb6a1398b0ba1302446efa79f21a0fc3049b1982e20233806d";

/// That pair's public half, as this crate reads it.
pub(crate) fn generated_2048_key() -> PublicKey {
    PublicKey::new(&unhex(GENERATED_2048_MODULUS), &unhex(RFC8448_PUBLIC))
        .expect("the generated key is odd, normalized, and has an odd exponent")
}
