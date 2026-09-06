// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two grammars a field is made of.

use crate::field::{is_tchar, is_token, is_value, same_name, trim};

#[test]
fn a_token_is_the_tchar_set_and_nothing_else() {
    for name in ["Content-Length", "X", "a!#$%&'*+-.^_`|~9", "ETag"] {
        assert!(is_token(name), "{name} is a token");
    }
    for name in [
        "", "a b", "a:b", "a(b", "a)b", "a,b", "a;b", "a/b", "a@b", "a\"b", "a[b", "a\\b", "a{b",
        "a\r\nb", "ä",
    ] {
        assert!(!is_token(name), "{name:?} is not a token");
    }
}

#[test]
fn a_value_is_printable_ascii_with_no_whitespace_at_its_ends() {
    for value in ["", "text/plain", "a b", "a\tb", "!~"] {
        assert!(is_value(value), "{value:?} is a value");
    }
    for value in [
        " leading",
        "trailing ",
        "\ttab",
        "tab\t",
        "a\rb",
        "a\nb",
        "a\0b",
        "ä",
    ] {
        assert!(!is_value(value), "{value:?} is not a value");
    }
}

#[test]
fn every_separator_of_the_grammar_is_out_of_the_token_set() {
    for byte in b"()<>@,;:\\\"/[]?={} \t" {
        assert!(!is_tchar(*byte), "{} is a separator", char::from(*byte));
    }
    for byte in b"!#$%&'*+-.^_`|~" {
        assert!(is_tchar(*byte), "{} is a tchar", char::from(*byte));
    }
    assert!(!is_tchar(0x7F));
    assert!(!is_tchar(0x00));
}

#[test]
fn a_name_is_compared_without_regard_to_case() {
    assert!(same_name("Content-Length", "content-length"));
    assert!(same_name("ETAG", "etag"));
    assert!(!same_name("Content-Length", "Content-Length "));
    assert!(!same_name("A", "B"));
}

#[test]
fn trimming_takes_spaces_and_tabs_and_nothing_else() {
    assert_eq!(trim("  value \t"), "value");
    assert_eq!(trim("\t\t"), "");
    assert_eq!(trim("value"), "value");
    assert_eq!(trim("a b"), "a b");
}
