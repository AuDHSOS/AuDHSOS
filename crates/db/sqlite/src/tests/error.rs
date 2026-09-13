// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::error`.

use crate::error::Error;

#[test]
fn every_refusal_says_which_rule_of_the_format_was_broken() {
    let cases = [
        (Error::Magic, "not a SQLite database"),
        (Error::Truncated, "hundred-byte header"),
        (Error::PageSize(300), "page size 300"),
        (Error::Reserved(200), "200 reserved bytes"),
        (Error::Fractions, "64, 32 and 32"),
        (Error::Encoding(9), "text encoding 9"),
        (Error::Page(7), "page 7"),
        (Error::PageKind(3), "3 is not a b-tree page type"),
        (Error::Overrun, "reaches past its page"),
        (Error::Varint, "varint runs past"),
        (Error::SerialType(10), "serial type 10"),
        (Error::Depth, "deeper than this crate walks"),
        (Error::Overflow(5), "overflow chain at page 5"),
    ];
    for (error, expected) in cases {
        let said = format!("{error}");
        assert!(said.contains(expected), "{error:?} said `{said}`");
    }
}

#[test]
fn a_refusal_is_compared_by_what_it_refuses() {
    assert_eq!(Error::Page(1), Error::Page(1));
    assert_ne!(Error::Page(1), Error::Page(2));
    assert_ne!(Error::Overrun, Error::Varint);
    // Copied rather than moved, so a refusal can be answered twice.
    let error = Error::Magic;
    let copy = error;
    assert_eq!(format!("{error:?}"), format!("{copy:?}"));
}
