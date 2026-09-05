// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::dictionary`.

use crate::dictionary::{Compare, Compares, MAX_WORD, TABLE_SIZE, Word, entry_from_compare};
use crate::rng::Rng;

#[test]
fn a_word_keeps_its_bytes_and_truncates_what_will_not_fit() {
    let word = Word::new(&[1, 2, 3]);
    assert_eq!(word.as_slice(), &[1, 2, 3]);
    assert_eq!(word.len(), 3);
    assert!(!word.is_empty());
    let empty = Word::new(&[]);
    assert!(empty.is_empty());
    assert!(empty.as_slice().is_empty());
    let long = Word::new(&[7u8; MAX_WORD + 10]);
    assert_eq!(long.len(), MAX_WORD);
    assert!(long.as_slice().iter().all(|byte| *byte == 7));
}

#[test]
fn a_table_remembers_what_it_was_given_and_wraps_its_index() {
    let mut table = Compares::new(4);
    assert_eq!(table.width(), 4);
    table.insert(0x1234, 0x1200);
    let slot = usize::try_from(0x1234u64 ^ 0x1200u64).unwrap() % TABLE_SIZE;
    assert_eq!(
        table.get(slot),
        Compare {
            left: 0x1234,
            right: 0x1200
        }
    );
    assert_eq!(table.get(slot), table.get(slot + TABLE_SIZE));
}

#[test]
fn an_empty_table_answers_with_nothing() {
    let table = Compares::new(8);
    assert_eq!(table.width(), 8);
    assert_eq!(table.get(3), Compare::default());
}

#[test]
fn an_entry_points_at_the_place_the_value_it_replaces_was_found() {
    let mut rng = Rng::new(5);
    let data = [0u8, 0, 0x30, 0x00, 0x00, 0x00, 9, 9];
    let compare = Compare {
        left: 0x30,
        right: 0x31,
    };
    let mut hinted = 0usize;
    for _ in 0..400 {
        let entry = entry_from_compare(&mut rng, compare, 4, &data);
        assert!(!entry.word.is_empty());
        assert_eq!(entry.word.len(), 4);
        if let Some(at) = entry.position {
            assert!(at < data.len());
            hinted += 1;
        }
    }
    assert!(hinted > 0, "no entry ever found the value in the input");
}

#[test]
fn an_entry_whose_value_is_nowhere_carries_no_place() {
    let mut rng = Rng::new(6);
    let compare = Compare {
        left: 0x1122_3344_5566_7788,
        right: 0x8877_6655_4433_2211,
    };
    for _ in 0..64 {
        let entry = entry_from_compare(&mut rng, compare, 8, &[]);
        assert_eq!(entry.position, None);
        assert_eq!(entry.word.len(), 8);
    }
}
