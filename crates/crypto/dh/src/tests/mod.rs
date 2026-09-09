// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.

mod group;
mod modp;

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

/// Renders bytes as lower-case hexadecimal, so that a failing vector
/// prints the two values side by side.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut rendered = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(rendered, "{byte:02x}");
    }
    rendered
}
