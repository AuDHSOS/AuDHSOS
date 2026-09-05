// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The generators: what they produce is what the crate is meant to read.

use core::cell::Cell;

use test_support::property::check;

use crate::pem::decode;
use crate::strategies::{any_block, any_label, any_payload, any_pem_bytes, write_block};

#[test]
fn a_generated_block_is_one_the_parser_accepts() {
    check("the generated block parses", &any_block(), |text| {
        let mut out = vec![0u8; text.len()];
        decode(text, &mut out)
            .map(|_| ())
            .map_err(|error| format!("{error}: {:?}", String::from_utf8_lossy(text)))
    });
}

#[test]
fn a_generated_label_and_payload_make_a_block() {
    check(
        "a generated label and payload write a block",
        &any_label(),
        |label| {
            let text = write_block(label, &[1, 2, 3, 4, 5]);
            let mut out = [0u8; 32];
            let block = decode(&text, &mut out).map_err(|error| error.to_string())?;
            if block.label == *label && block.bytes == [1, 2, 3, 4, 5] {
                Ok(())
            } else {
                Err(format!("{label:?} came back as {:?}", block.label))
            }
        },
    );
}

#[test]
fn a_generated_payload_is_never_empty() {
    check("the generated payload has bytes", &any_payload(), |bytes| {
        if bytes.is_empty() {
            Err("an empty payload".to_owned())
        } else {
            Ok(())
        }
    });
}

#[test]
fn the_mutated_generator_reaches_both_answers() {
    // A generator that only ever produced rejects would test the first
    // rule and nothing after it, so this asserts that it reaches an
    // accepted block as well as a refused one.
    let accepted = Cell::new(0u32);
    let refused = Cell::new(0u32);
    check(
        "the mutated generator reaches both",
        &any_pem_bytes(),
        |text| {
            let mut out = vec![0u8; text.len().max(1)];
            let counter = if decode(text, &mut out).is_ok() {
                &accepted
            } else {
                &refused
            };
            counter.set(counter.get().saturating_add(1));
            Ok(())
        },
    );
    assert!(
        accepted.get() > 0,
        "the generator never produced a valid block"
    );
    assert!(
        refused.get() > 0,
        "the generator never produced an invalid one"
    );
}
