// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::error`.

use crate::error::Error;
use std::collections::HashSet;

#[test]
fn every_variant_round_trips_through_its_code() {
    for &error in Error::ALL {
        assert_eq!(Error::from_code(error.code()), Some(error));
        assert_eq!(Error::try_from(error.code()), Ok(error));
    }
}

#[test]
fn codes_are_unique_and_non_zero() {
    let codes: HashSet<u32> = Error::ALL.iter().map(|e| e.code()).collect();
    assert_eq!(codes.len(), Error::ALL.len());
    assert!(!codes.contains(&0));
}

#[test]
fn code_zero_is_not_an_error() {
    assert_eq!(Error::from_code(0), None);
    assert_eq!(Error::try_from(0), Err(Error::InvalidArgument));
}

#[test]
fn unknown_code_is_rejected() {
    let highest = Error::ALL.iter().map(|e| e.code()).max().unwrap();
    assert_eq!(Error::from_code(highest + 1), None);
    assert_eq!(Error::from_code(u32::MAX), None);
}

#[test]
fn display_uses_the_table_message() {
    assert_eq!(
        Error::InvalidHandle.to_string(),
        Error::InvalidHandle.message()
    );
    for &error in Error::ALL {
        assert!(!error.message().is_empty());
    }
}

#[test]
fn codes_are_dense_from_one() {
    let mut codes: Vec<u32> = Error::ALL.iter().map(|e| e.code()).collect();
    codes.sort_unstable();
    let expected: Vec<u32> = (1..=u32::try_from(Error::ALL.len()).unwrap()).collect();
    assert_eq!(codes, expected);
}

#[test]
fn a_cancelled_operation_has_a_code_of_its_own() {
    assert_eq!(Error::Cancelled.code(), 25);
    assert_eq!(Error::from_code(25), Some(Error::Cancelled));
}
