// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::utf8` where a database cannot reach it.
//!
//! What a database reaches is checked in `tests/db.rs`, against the rows
//! SQLite answers for files it wrote itself. What is here is the text a
//! file may hold and the shell will not write: a pair with only one
//! half, a sequence that is too short, and the characters the reader
//! answers with instead.

#![allow(clippy::arithmetic_side_effects)]

use crate::utf8::{REPLACEMENT, count, from_utf16, read, skip, to_utf16, units, write};

/// The text as a string, for comparing.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_pair_with_one_half_missing_is_read_as_far_as_it_goes() {
    // `sqlite3VdbeMemTranslate` joins a surrogate with whatever code
    // unit follows it, and where none follows it takes it as it is.
    // A lone surrogate is not text a string holds, so it is compared as
    // the bytes it is written as.
    assert_eq!(from_utf16(&[0x00, 0xd8], false), [0xed, 0xa0, 0x80]);
    assert_eq!(from_utf16(&[0xd8, 0x00], true), [0xed, 0xa0, 0x80]);
    // A pair, both ways round.
    assert_eq!(
        text(&from_utf16(&[0x00, 0xd8, 0x00, 0xdc], false)),
        "\u{10000}"
    );
    assert_eq!(
        text(&from_utf16(&[0xd8, 0x00, 0xdc, 0x00], true)),
        "\u{10000}"
    );
    // An odd number of bytes ends where the last whole unit does.
    assert_eq!(text(&from_utf16(&[0x61, 0x00, 0x62], false)), "a");
    assert_eq!(from_utf16(&[], false), b"");
}

#[test]
fn text_is_written_as_the_units_it_is_made_of() {
    assert_eq!(to_utf16(b"a", false), [0x61, 0x00]);
    assert_eq!(to_utf16(b"a", true), [0x00, 0x61]);
    assert_eq!(
        to_utf16("\u{10000}".as_bytes(), false),
        [0x00, 0xd8, 0x00, 0xdc]
    );
    assert_eq!(units(b"az").collect::<Vec<u32>>(), [0x61, 0x7a]);
    assert_eq!(
        units("\u{10000}".as_bytes()).collect::<Vec<u32>>(),
        [0xd800, 0xdc00]
    );
    // Every string this crate holds is written and read back as itself.
    for case in ["", "a", "äöü", "\u{10000}", "z\u{e000}\u{fffd}"] {
        assert_eq!(
            text(&from_utf16(&to_utf16(case.as_bytes(), false), false)),
            case
        );
        assert_eq!(
            text(&from_utf16(&to_utf16(case.as_bytes(), true), true)),
            case
        );
    }
}

#[test]
fn a_sequence_that_is_not_one_reads_as_the_character_for_that() {
    // Overlong, a surrogate, and the two characters that are not.
    assert_eq!(read(&[0xc0, 0x80], 0).0, REPLACEMENT);
    assert_eq!(read(&[0xed, 0xa0, 0x80], 0).0, REPLACEMENT);
    assert_eq!(read(&[0xef, 0xbf, 0xbe], 0).0, REPLACEMENT);
    assert_eq!(read(&[0xef, 0xbf, 0xbf], 0).0, REPLACEMENT);
    // A continuation byte with nothing before it is a character of its
    // own, which is what makes the reader total.
    assert_eq!(read(&[0x80], 0), (0x80, 1));
    assert_eq!(read(b"", 0), (0, 0));
    assert_eq!(read(b"a", 1), (0, 1));
    // Skipping does not decode, so it passes the same bytes.
    assert_eq!(skip(&[0xed, 0xa0, 0x80], 0), 3);
    assert_eq!(skip(b"ab", 0), 1);
    // Counting stops at a NUL, as a C string does.
    assert_eq!(count(b"abc"), 3);
    assert_eq!(count("äöü".as_bytes()), 3);
    assert_eq!(count(b"ab\0cd"), 2);
    assert_eq!(count(b""), 0);
}

#[test]
fn every_character_is_written_in_as_few_bytes_as_it_takes() {
    let written = |value: u32| {
        let mut out = Vec::new();
        write(&mut out, value);
        out
    };
    assert_eq!(written(0x41), b"A");
    assert_eq!(written(0), [0]);
    assert_eq!(written(0x7f).len(), 1);
    assert_eq!(written(0x80).len(), 2);
    assert_eq!(written(0x7ff).len(), 2);
    assert_eq!(written(0x800).len(), 3);
    assert_eq!(written(0xffff).len(), 3);
    assert_eq!(written(0x1_0000).len(), 4);
    assert_eq!(written(0x10_ffff).len(), 4);
}
