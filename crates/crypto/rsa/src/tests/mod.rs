// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.

mod bigint;
mod key;
mod pkcs1;

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
