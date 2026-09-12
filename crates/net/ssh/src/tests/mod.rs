// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.

mod cipher;
mod error;
mod exchange;
mod ident;
mod kex;
mod keys;
mod msg;
mod packet;
mod wire;

/// One `string` of RFC 4251, section 5, built a second time so that a
/// hash test does not share an encoder with what it checks.
pub(crate) fn ssh_string(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&u32::try_from(value.len()).unwrap_or_default().to_be_bytes());
    out.extend_from_slice(value);
    out
}

/// One `mpint` of that section, from an unsigned big-endian value.
pub(crate) fn ssh_mpint(value: &[u8]) -> Vec<u8> {
    let mut digits: Vec<u8> = value
        .iter()
        .copied()
        .skip_while(|byte| *byte == 0)
        .collect();
    if digits.first().is_some_and(|byte| byte & 0x80 != 0) {
        digits.insert(0, 0x00);
    }
    ssh_string(&digits)
}

/// The bytes of a hexadecimal string, for the vectors that are written
/// that way in the documents they come from.
pub(crate) fn unhex(text: &str) -> Vec<u8> {
    let digits: Vec<u32> = text.chars().filter_map(|c| c.to_digit(16)).collect();
    let (pairs, _) = digits.as_chunks::<2>();
    pairs
        .iter()
        .map(|[high, low]| u8::try_from(high.wrapping_mul(16).wrapping_add(*low)).unwrap_or(0))
        .collect()
}
