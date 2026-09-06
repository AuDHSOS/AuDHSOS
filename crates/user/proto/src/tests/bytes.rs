// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::bytes`.

use crate::bytes::Bytes;
use crate::label::ProtoError;

/// The width the tests work at.
type Short = Bytes<4>;

#[test]
fn an_empty_string_holds_nothing() {
    let value = Short::empty();
    assert!(value.is_empty());
    assert_eq!(value.len(), 0);
    assert_eq!(value.as_bytes(), b"");
    assert_eq!(Short::capacity(), 4);
    assert_eq!(Short::default(), value);
}

#[test]
fn what_was_put_in_comes_back() {
    let value = Short::new(b"abc").unwrap();
    assert_eq!(value.as_bytes(), b"abc");
    assert_eq!(value.len(), 3);
    assert!(!value.is_empty());
}

#[test]
fn a_string_that_fills_the_array_fits() {
    let value = Short::new(b"abcd").unwrap();
    assert_eq!(value.as_bytes(), b"abcd");
}

#[test]
fn one_byte_more_than_the_array_holds_is_refused() {
    assert_eq!(
        Short::new(b"abcde").unwrap_err(),
        ProtoError::TooLong {
            len: 5,
            capacity: 4,
        }
    );
}

#[test]
fn two_strings_are_equal_when_their_bytes_are() {
    assert_eq!(Short::new(b"ab").unwrap(), Short::new(b"ab").unwrap());
    assert_ne!(Short::new(b"ab").unwrap(), Short::new(b"abc").unwrap());
    // The array behind a short string still holds the zeros it started
    // with, so equality has to be over the length and not over the array.
    assert_ne!(Short::new(b"ab").unwrap(), Short::empty());
}
