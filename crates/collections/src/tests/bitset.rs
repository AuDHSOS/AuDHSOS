// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `BitSet` at its edges: bit zero, a word boundary, the last bit, and one
//! beyond it.

use std::collections::BTreeSet;

use test_support::generators::BoxGen;
use test_support::model::{ModelTest, run_model_test};

use crate::bitset::BitSet;
use crate::error::CollectionError;
use crate::strategies::{BitOp, any_bit_op};

/// Words of the set the model test runs against: two, so that the tests
/// cross a word boundary.
const WORDS: usize = 2;

/// Bits of that set.
const BITS: usize = WORDS * 64;

#[test]
fn a_new_set_is_empty_and_reports_its_size() {
    let set: BitSet<2> = BitSet::new();
    assert_eq!(set.bits(), 128);
    assert_eq!(BitSet::<2>::BITS, 128);
    assert!(set.is_empty());
    assert_eq!(set.count(), 0);
    assert_eq!(set.first_set(), None);
    assert_eq!(set.first_clear(), Some(0));
    let default: BitSet<2> = BitSet::default();
    assert_eq!(default, set);
    assert!(!format!("{default:?}").is_empty());
}

#[test]
fn a_full_set_has_every_bit() {
    let set: BitSet<2> = BitSet::full();
    assert!(!set.is_empty());
    assert_eq!(set.count(), 128);
    assert_eq!(set.first_set(), Some(0));
    assert_eq!(set.first_clear(), None);
    assert_eq!(set.test(127), Ok(true));
}

#[test]
fn setting_clearing_and_testing_work_at_the_edges() {
    let mut set: BitSet<2> = BitSet::new();
    // Bit zero, the last bit of the first word, the first of the second,
    // and the last of the set.
    for index in [0, 63, 64, 127] {
        assert_eq!(set.set(index), Ok(false), "{index}");
        assert_eq!(set.test(index), Ok(true), "{index}");
        assert_eq!(set.set(index), Ok(true), "{index}");
    }
    assert_eq!(set.count(), 4);
    for index in [0, 63, 64, 127] {
        assert_eq!(set.clear(index), Ok(true), "{index}");
        assert_eq!(set.test(index), Ok(false), "{index}");
        assert_eq!(set.clear(index), Ok(false), "{index}");
    }
    assert!(set.is_empty());
}

#[test]
fn an_index_at_or_beyond_the_size_is_an_error() {
    let mut set: BitSet<2> = BitSet::new();
    assert_eq!(set.test(128), Err(CollectionError::Index(128)));
    assert_eq!(set.set(128), Err(CollectionError::Index(128)));
    assert_eq!(set.clear(128), Err(CollectionError::Index(128)));
    assert_eq!(
        set.test(usize::MAX),
        Err(CollectionError::Index(usize::MAX))
    );
    assert_eq!(set.test(127), Ok(false));
}

#[test]
fn a_set_without_words_holds_no_bit() {
    let mut set: BitSet<0> = BitSet::new();
    assert_eq!(set.bits(), 0);
    assert!(set.is_empty());
    assert_eq!(set.first_set(), None);
    assert_eq!(set.first_clear(), None);
    assert_eq!(set.set(0), Err(CollectionError::Index(0)));
    assert_eq!(BitSet::<0>::full().count(), 0);
}

#[test]
fn the_first_set_and_first_clear_bits_are_found_across_words() {
    let mut set: BitSet<2> = BitSet::new();
    assert_eq!(set.first_set(), None);
    set.set(70).expect("in range");
    assert_eq!(set.first_set(), Some(70));
    set.set(5).expect("in range");
    assert_eq!(set.first_set(), Some(5));

    let mut full: BitSet<2> = BitSet::full();
    assert_eq!(full.first_clear(), None);
    full.clear(100).expect("in range");
    assert_eq!(full.first_clear(), Some(100));
    full.clear(64).expect("in range");
    assert_eq!(full.first_clear(), Some(64));
    full.clear(0).expect("in range");
    assert_eq!(full.first_clear(), Some(0));
}

#[test]
fn a_whole_first_word_that_is_full_sends_the_search_into_the_second() {
    let mut set: BitSet<2> = BitSet::new();
    for index in 0..64 {
        set.set(index).expect("in range");
    }
    assert_eq!(set.first_clear(), Some(64));
    assert_eq!(set.count(), 64);
    set.clear_all();
    assert!(set.is_empty());
    assert_eq!(set.first_set(), None);
}

/// The set against a `BTreeSet` of the indices that are set.
struct BitModel;

impl ModelTest for BitModel {
    type Op = BitOp;
    type Sut = BitSet<WORDS>;
    type Model = BTreeSet<usize>;

    fn generator(&self) -> BoxGen<BitOp> {
        any_bit_op(BITS)
    }

    fn new_sut(&self) -> BitSet<WORDS> {
        BitSet::new()
    }

    fn new_model(&self) -> BTreeSet<usize> {
        BTreeSet::new()
    }

    fn step(
        &self,
        sut: &mut BitSet<WORDS>,
        model: &mut BTreeSet<usize>,
        op: &BitOp,
    ) -> Result<(), String> {
        match *op {
            BitOp::Set(index) => {
                let expected = if index < BITS {
                    Ok(model.contains(&index))
                } else {
                    Err(CollectionError::Index(index))
                };
                if sut.set(index) != expected {
                    return Err(format!("set {index} against {expected:?}"));
                }
                if index < BITS {
                    model.insert(index);
                }
            }
            BitOp::Clear(index) => {
                let expected = if index < BITS {
                    Ok(model.contains(&index))
                } else {
                    Err(CollectionError::Index(index))
                };
                if sut.clear(index) != expected {
                    return Err(format!("clear {index} against {expected:?}"));
                }
                model.remove(&index);
            }
            BitOp::Test(index) => {
                let expected = if index < BITS {
                    Ok(model.contains(&index))
                } else {
                    Err(CollectionError::Index(index))
                };
                if sut.test(index) != expected {
                    return Err(format!("test {index} against {expected:?}"));
                }
            }
            BitOp::FirstSet => {
                let expected = model.first().copied();
                if sut.first_set() != expected {
                    return Err(format!("first_set against {expected:?}"));
                }
            }
            BitOp::FirstClear => {
                let expected = (0..BITS).find(|index| !model.contains(index));
                if sut.first_clear() != expected {
                    return Err(format!("first_clear against {expected:?}"));
                }
            }
        }
        if sut.count() != model.len() {
            return Err(format!("count {} against {}", sut.count(), model.len()));
        }
        if sut.is_empty() != model.is_empty() {
            return Err("emptiness disagrees".to_owned());
        }
        Ok(())
    }
}

#[test]
fn model_a_bit_set_agrees_with_a_set_of_indices() {
    run_model_test("bitset_model", &BitModel, 96);
}
