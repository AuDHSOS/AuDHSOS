// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.

mod hkdf;
mod hmac;
mod sha256;
mod sha512;

/// Renders bytes as lower-case hexadecimal, so that a failing vector prints
/// the two digests side by side instead of two arrays of numbers.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut rendered = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(rendered, "{byte:02x}");
    }
    rendered
}
