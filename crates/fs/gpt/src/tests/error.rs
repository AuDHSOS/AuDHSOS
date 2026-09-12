// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::error`.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use crate::error::Error;

/// One of every refusal, so that each renders and none renders empty.
const EVERY: [Error; 15] = [
    Error::Device(7),
    Error::Lba(1 << 40),
    Error::NotProtective,
    Error::Signature,
    Error::Revision(0x0002_0000),
    Error::HeaderSize(4),
    Error::HeaderChecksum,
    Error::MyLba(9),
    Error::EntrySize(96),
    Error::ArrayRange,
    Error::ArrayChecksum,
    Error::Usable(40, 30),
    Error::TooSmall(2),
    Error::Name,
    Error::Partition(1024),
];

#[test]
fn every_refusal_renders_a_sentence_of_its_own() {
    let mut rendered: Vec<String> = EVERY.iter().map(ToString::to_string).collect();
    assert!(rendered.iter().all(|text| !text.is_empty()));
    rendered.sort();
    let count = rendered.len();
    rendered.dedup();
    assert_eq!(rendered.len(), count, "two refusals read the same");
    assert_eq!(
        Error::Space(200).to_string(),
        "200 entries do not fit the array"
    );
}

#[test]
fn a_refusal_is_an_error() {
    let error: &dyn core::error::Error = &Error::Signature;
    assert!(error.source().is_none());
}
