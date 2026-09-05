// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generators for property tests. Available behind the feature
//! `test-strategies` and in this crate's own tests.
//!
//! [`any_pem_bytes`] produces texts that are *nearly* blocks: a valid one,
//! then a handful of bytes replaced and sometimes a truncation. A
//! generator of purely random bytes would spend every case in
//! [`EncodingError::MissingBegin`](crate::EncodingError::MissingBegin) and
//! would never reach the rules that matter.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use test_support::generators::{BoxGen, Generator, bool, bytes, one_of, pair, range, vec};

use crate::pem;

/// The labels a block of this project carries.
const LABELS: [&str; 4] = ["CERTIFICATE", "PRIVATE KEY", "PUBLIC KEY", "X"];

/// A payload of one to two hundred bytes.
#[must_use]
pub fn any_payload() -> BoxGen<Vec<u8>> {
    bytes(1..=200).boxed()
}

/// One of the labels this project writes.
#[must_use]
pub fn any_label() -> BoxGen<&'static str> {
    one_of(LABELS.to_vec()).boxed()
}

/// A well-formed block.
#[must_use]
pub fn any_block() -> BoxGen<Vec<u8>> {
    pair(any_label(), any_payload())
        .map(|(label, payload)| write_block(label, &payload))
        .boxed()
}

/// A block with up to four bytes replaced and sometimes truncated, so that
/// most cases are near-valid and the parser is driven through its rules
/// rather than through its first one.
#[must_use]
pub fn any_pem_bytes() -> BoxGen<Vec<u8>> {
    let mutations = vec(pair(range(0usize..=400), range(0u8..=u8::MAX)), 0..=4);
    pair(any_block(), pair(mutations, bool()))
        .map(|(block, (mutations, truncate))| {
            let mut text = block;
            for (index, value) in mutations {
                let index = index.checked_rem(text.len()).unwrap_or(0);
                if let Some(slot) = text.get_mut(index) {
                    *slot = value;
                }
            }
            if truncate && !text.is_empty() {
                let keep = text.len().wrapping_div(2);
                text.truncate(keep);
            }
            text
        })
        .boxed()
}

/// The canonical text of a block, as [`pem::encode`] writes it.
#[must_use]
pub fn write_block(label: &str, payload: &[u8]) -> Vec<u8> {
    let needed = pem::encoded_len(label, payload.len()).unwrap_or(0);
    let mut text = vec![0u8; needed];
    let written = pem::encode(label, payload, &mut text).unwrap_or(0);
    text.truncate(written);
    text
}
