// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::tree`.

use crate::tree::{Tree, sequence, zip};
use std::rc::Rc;

fn towards_zero(value: u32) -> Tree<u32> {
    Tree::new(value, move || (0..value).map(towards_zero).collect())
}

#[test]
fn leaf_has_no_candidates() {
    assert!(Tree::leaf(5u8).shrinks().is_empty());
    assert_eq!(*Tree::leaf(5u8).value(), 5);
    assert_eq!(Tree::leaf(5u8).into_value(), 5);
}

#[test]
fn map_applies_to_value_and_candidates() {
    let doubled = towards_zero(3).map(Rc::new(|v| v * 2));
    assert_eq!(*doubled.value(), 6);
    let children: Vec<u32> = doubled.shrinks().iter().map(|t| *t.value()).collect();
    assert_eq!(children, vec![0, 2, 4]);
}

#[test]
fn filter_drops_value_and_candidates_that_fail() {
    let even = towards_zero(4).filter(Rc::new(|v| v % 2 == 0)).unwrap();
    let children: Vec<u32> = even.shrinks().iter().map(|t| *t.value()).collect();
    assert_eq!(children, vec![0, 2]);
    assert!(towards_zero(3).filter(Rc::new(|v| v % 2 == 0)).is_none());
}

#[test]
fn zip_shrinks_left_then_right() {
    let pair = zip(towards_zero(1), towards_zero(1));
    let children: Vec<(u32, u32)> = pair.shrinks().iter().map(|t| *t.value()).collect();
    assert_eq!(children, vec![(0, 1), (1, 0)]);
}

#[test]
fn sequence_removes_chunks_then_shrinks_elements() {
    let element = towards_zero(1);
    let seq = sequence(
        vec![element.clone(), element.clone(), element.clone(), element],
        0,
    );
    let children: Vec<Vec<u32>> = seq.shrinks().iter().map(|t| t.value().clone()).collect();
    assert_eq!(children.first(), Some(&vec![1, 1]));
    assert!(children.contains(&vec![1, 1, 1]));
    assert!(children.contains(&vec![0, 1, 1, 1]));
    assert!(children.contains(&vec![1, 1, 1, 0]));
}

#[test]
fn sequence_respects_the_minimum_length() {
    let seq = sequence(vec![towards_zero(0), towards_zero(0)], 2);
    assert!(seq.shrinks().is_empty());
    let empty: Tree<Vec<u32>> = sequence(vec![], 0);
    assert!(empty.value().is_empty());
    assert!(empty.shrinks().is_empty());
}

#[test]
fn debug_shows_the_value() {
    assert!(format!("{:?}", Tree::leaf(3u8)).contains("value: 3"));
}
