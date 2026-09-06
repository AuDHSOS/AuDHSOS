// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a header name and a header value may be made of.
//!
//! The two grammars are RFC 9110, section 5.6.2 for the name — the
//! `tchar` set, which is the printable ASCII minus the separators — and
//! RFC 9110, section 5.5 for the value, which is visible ASCII, the
//! space, and the horizontal tab, and nothing else. A byte outside them
//! is refused on the way in and on the way out, so a header this crate
//! writes is one it would read back.
//!
//! The reason the outbound check matters as much as the inbound one is
//! that a value carrying a carriage return is a value that ends the field
//! and begins another. A client that writes one has let its caller write
//! a header of its own choosing, which is response splitting from the
//! wrong end.

/// Whether `byte` is a `tchar` (RFC 9110, section 5.6.2).
#[must_use]
pub const fn is_tchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// Whether `text` is a token: one or more `tchar` and nothing else.
#[must_use]
pub fn is_token(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(is_tchar)
}

/// Whether `text` is a field value: visible ASCII, spaces and horizontal
/// tabs, with no leading or trailing whitespace.
#[must_use]
pub fn is_value(text: &str) -> bool {
    if text.bytes().any(|byte| !is_value_byte(byte)) {
        return false;
    }
    !matches!(text.as_bytes(), [b' ' | b'\t', ..] | [.., b' ' | b'\t'])
}

/// Whether `byte` may stand in a field value.
///
/// RFC 9110, section 5.5 allows `obs-text` — the bytes from `0x80` up —
/// and deprecates it in the same breath. It is refused here, and what
/// that buys is that every value this crate hands out is ASCII and
/// therefore text, with no place where a caller has to decide what a byte
/// above 127 meant.
const fn is_value_byte(byte: u8) -> bool {
    byte == b'\t' || byte.is_ascii_graphic() || byte == b' '
}

/// Whether `left` and `right` are the same field name, which is compared
/// without regard to case (RFC 9110, section 5.1).
#[must_use]
pub const fn same_name(left: &str, right: &str) -> bool {
    left.len() == right.len() && left.as_bytes().eq_ignore_ascii_case(right.as_bytes())
}

/// `text` without the optional whitespace RFC 9110, section 5.5 allows
/// around a field value.
#[must_use]
pub fn trim(text: &str) -> &str {
    text.trim_matches([' ', '\t'])
}
