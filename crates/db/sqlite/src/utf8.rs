// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reading UTF-8 the way SQLite reads it.
//!
//! `src/utf.c` decodes leniently: a sequence that is too short, a
//! surrogate and the two non-characters all come back as the
//! replacement character rather than as a refusal, and a continuation
//! byte with nothing in front of it is a character of its own. A reader
//! that refuses where SQLite does not would answer a different length
//! and a different `substr`, so this is that reader and not a correct
//! one.

/// The first six bits of a lead byte, indexed by the byte less `0xc0`.
const LEAD: [u8; 64] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x00, 0x01, 0x02, 0x03, 0x00, 0x01, 0x00, 0x00,
];

/// The character SQLite answers where the bytes are not one.
pub const REPLACEMENT: u32 = 0xfffd;

/// The character at `at`, and where the one after it begins.
///
/// This is `sqlite3Utf8Read`. Past the end it answers nothing, which is
/// the NUL a C string ends with.
#[must_use]
pub fn read(bytes: &[u8], at: usize) -> (u32, usize) {
    let Some(lead) = bytes.get(at).copied() else {
        return (0, at);
    };
    let mut next = at.saturating_add(1);
    if lead < 0xc0 {
        return (u32::from(lead), next);
    }
    let index = usize::from(lead.wrapping_sub(0xc0));
    let mut value = u32::from(LEAD.get(index).copied().unwrap_or(0));
    while bytes.get(next).is_some_and(|byte| byte & 0xc0 == 0x80) {
        let byte = bytes.get(next).copied().unwrap_or(0);
        value = (value << 6).wrapping_add(u32::from(byte & 0x3f));
        next = next.saturating_add(1);
    }
    if value < 0x80 || value & 0xffff_f800 == 0xd800 || value & 0xffff_fffe == 0xfffe {
        value = REPLACEMENT;
    }
    (value, next)
}

/// Where the character after the one at `at` begins, without decoding it.
///
/// This is `SQLITE_SKIP_UTF8`.
#[must_use]
pub fn skip(bytes: &[u8], at: usize) -> usize {
    let mut next = at.saturating_add(1);
    if bytes.get(at).copied().unwrap_or(0) >= 0xc0 {
        while bytes.get(next).is_some_and(|byte| byte & 0xc0 == 0x80) {
            next = next.saturating_add(1);
        }
    }
    next
}

/// How many characters `bytes` holds, up to the first NUL.
#[must_use]
pub fn count(bytes: &[u8]) -> usize {
    let mut at = 0;
    let mut characters: usize = 0;
    while bytes.get(at).is_some_and(|byte| *byte != 0) {
        at = skip(bytes, at);
        characters = characters.saturating_add(1);
    }
    characters
}

/// A character written out, appended to `out`.
pub fn write(out: &mut alloc::vec::Vec<u8>, value: u32) {
    let byte = |bits: u32| u8::try_from(bits & 0xff).unwrap_or(0);
    if value < 0x80 {
        out.push(byte(value));
    } else if value < 0x800 {
        out.push(0xc0_u8.wrapping_add(byte((value >> 6) & 0x1f)));
        out.push(0x80_u8.wrapping_add(byte(value & 0x3f)));
    } else if value < 0x1_0000 {
        out.push(0xe0_u8.wrapping_add(byte((value >> 12) & 0x0f)));
        out.push(0x80_u8.wrapping_add(byte((value >> 6) & 0x3f)));
        out.push(0x80_u8.wrapping_add(byte(value & 0x3f)));
    } else {
        out.push(0xf0_u8.wrapping_add(byte((value >> 18) & 0x07)));
        out.push(0x80_u8.wrapping_add(byte((value >> 12) & 0x3f)));
        out.push(0x80_u8.wrapping_add(byte((value >> 6) & 0x3f)));
        out.push(0x80_u8.wrapping_add(byte(value & 0x3f)));
    }
}
