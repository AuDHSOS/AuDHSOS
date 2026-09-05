// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::mutate`.
//!
//! The mutations are drawn, so a test of one of them in isolation would be
//! a test of the draw. What is checked here instead is what every mutation
//! must hold to whatever it drew, over enough rounds that every one of
//! them is reached: the input stays within its limit, a round that changed
//! nothing says so, and the sequence is written down.

use crate::dictionary::Word;
use crate::mutate::{Mutation, Mutator};
use crate::sancov::with_trace;

use super::GLOBALS;

/// Every mutation, so that a test can check none was left unreached.
const EVERY: [Mutation; 13] = [
    Mutation::EraseBytes,
    Mutation::InsertByte,
    Mutation::InsertRepeatedBytes,
    Mutation::ChangeByte,
    Mutation::ChangeBit,
    Mutation::ShuffleBytes,
    Mutation::ChangeAsciiInteger,
    Mutation::ChangeBinaryInteger,
    Mutation::CopyPart,
    Mutation::CrossOver,
    Mutation::ManualDictionary,
    Mutation::PersistentDictionary,
    Mutation::Compare,
];

#[test]
fn every_mutation_is_reached_and_none_of_them_breaks_the_limit() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    with_trace(|trace| {
        trace.compares4.insert(0x3132_3334, 0x3132_3335);
        trace.compares8.insert(0x4142_4344_4546_4748, 0);
    });
    let mut mutator = Mutator::new(1234);
    mutator.add_word(Word::new(b"BEGIN"));
    let cross = b"0123456789abcdefghij".to_vec();
    let mut used = Vec::new();
    for round in 0..40_000u32 {
        let mut input = b"12345678 A0\x01\x02\x03\x04 xyz".to_vec();
        let limit = 8 + (usize::try_from(round).unwrap_or(0) % 64);
        mutator.begin_round();
        let changed = with_trace(|trace| mutator.mutate(&mut input, limit, Some(&cross), trace));
        assert!(input.len() <= limit, "a mutation broke the limit");
        if changed {
            assert_eq!(mutator.sequence().len(), 1);
            if let Some(step) = mutator.sequence().first()
                && !used.contains(step)
            {
                used.push(*step);
            }
            mutator.reward();
        }
    }
    for step in EVERY {
        assert!(used.contains(&step), "{} was never drawn", step.name());
    }
    drop(guard);
}

#[test]
fn a_mutation_that_cannot_apply_leaves_the_input_alone() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut mutator = Mutator::new(9);
    let mut empty = Vec::new();
    mutator.begin_round();
    let changed = with_trace(|trace| mutator.mutate(&mut empty, 0, None, trace));
    assert!(!changed);
    assert!(empty.is_empty());
    assert!(mutator.sequence().is_empty());
    drop(guard);
}

#[test]
fn a_run_without_a_dictionary_or_a_second_input_still_mutates() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut mutator = Mutator::new(77);
    assert_eq!(mutator.words(), 0);
    let mut changes = 0usize;
    for _ in 0..2000 {
        let mut input = b"hello".to_vec();
        mutator.begin_round();
        if with_trace(|trace| mutator.mutate(&mut input, 16, None, trace)) {
            changes += 1;
        }
    }
    assert!(
        changes > 1500,
        "only {changes} of 2000 rounds changed anything"
    );
    drop(guard);
}

#[test]
fn a_word_that_paid_off_is_kept_and_drawn_again() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut mutator = Mutator::new(4242);
    mutator.add_word(Word::new(b"MAGIC"));
    let mut pasted = 0usize;
    for _ in 0..20_000 {
        let mut input = b"..........".to_vec();
        mutator.begin_round();
        let changed = with_trace(|trace| mutator.mutate(&mut input, 32, None, trace));
        if changed && mutator.sequence().first() == Some(&Mutation::PersistentDictionary) {
            pasted += 1;
        }
        if changed {
            mutator.reward();
        }
    }
    assert!(
        pasted > 0,
        "the dictionary of paid-off words was never drawn"
    );
    drop(guard);
}

#[test]
fn every_mutation_has_a_name() {
    for step in EVERY {
        assert!(!step.name().is_empty());
    }
}
