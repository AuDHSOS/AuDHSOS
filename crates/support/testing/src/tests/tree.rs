// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::tree`.

use crate::tree::{Tree, sequence, zip};
use std::cell::Cell;
use std::rc::Rc;

fn towards_zero(value: u32) -> Tree<u32> {
    Tree::new(value, move || (0..value).map(towards_zero))
}

#[test]
fn leaf_has_no_candidates() {
    assert!(Tree::leaf(5u8).shrinks().next().is_none());
    assert_eq!(*Tree::leaf(5u8).value(), 5);
    assert_eq!(Tree::leaf(5u8).into_value(), 5);
}

#[test]
fn map_applies_to_value_and_candidates() {
    let doubled = towards_zero(3).map(Rc::new(|v| v * 2));
    assert_eq!(*doubled.value(), 6);
    let children: Vec<u32> = doubled.shrinks().map(|t| *t.value()).collect();
    assert_eq!(children, vec![0, 2, 4]);
}

#[test]
fn filter_drops_value_and_candidates_that_fail() {
    let even = towards_zero(4).filter(Rc::new(|v| v % 2 == 0)).unwrap();
    let children: Vec<u32> = even.shrinks().map(|t| *t.value()).collect();
    assert_eq!(children, vec![0, 2]);
    assert!(towards_zero(3).filter(Rc::new(|v| v % 2 == 0)).is_none());
}

#[test]
fn zip_shrinks_left_then_right() {
    let pair = zip(towards_zero(1), towards_zero(1));
    let children: Vec<(u32, u32)> = pair.shrinks().map(|t| *t.value()).collect();
    assert_eq!(children, vec![(0, 1), (1, 0)]);
}

#[test]
fn sequence_removes_chunks_then_shrinks_elements() {
    let element = towards_zero(1);
    let seq = sequence(
        vec![element.clone(), element.clone(), element.clone(), element],
        0,
    );
    let children: Vec<Vec<u32>> = seq.shrinks().map(Tree::into_value).collect();
    assert_eq!(children.first(), Some(&vec![1, 1]));
    assert!(children.contains(&vec![1, 1, 1]));
    assert!(children.contains(&vec![0, 1, 1, 1]));
    assert!(children.contains(&vec![1, 1, 1, 0]));
}

#[test]
fn sequence_respects_the_minimum_length() {
    let seq = sequence(vec![towards_zero(0), towards_zero(0)], 2);
    assert!(seq.shrinks().next().is_none());
    let empty: Tree<Vec<u32>> = sequence(vec![], 0);
    assert!(empty.value().is_empty());
    assert!(empty.shrinks().next().is_none());
}

#[test]
fn sequence_builds_candidates_only_when_requested() {
    let calls = Rc::new(Cell::new(0));
    let elements = (1..=4)
        .map(|value| {
            let calls = Rc::clone(&calls);
            Tree::new(value, move || {
                calls.set(calls.get() + 1);
                std::iter::once(Tree::leaf(0))
            })
        })
        .collect();
    let seq = sequence(elements, 0);
    let mut children = seq.shrinks();
    assert_eq!(calls.get(), 0);
    assert_eq!(children.next().unwrap().value(), &vec![3, 4]);
    assert_eq!(calls.get(), 0);
    for _ in 0..5 {
        children.next().unwrap();
    }
    assert_eq!(calls.get(), 0);
    assert_eq!(children.next().unwrap().value(), &vec![0, 2, 3, 4]);
    assert_eq!(calls.get(), 1);
}

#[test]
fn sequence_clones_only_the_requested_chunk() {
    struct Counted(Rc<Cell<usize>>);

    impl Clone for Counted {
        fn clone(&self) -> Self {
            self.0.set(self.0.get() + 1);
            Self(Rc::clone(&self.0))
        }
    }

    let clones = Rc::new(Cell::new(0));
    let seq = sequence(
        (0..64)
            .map(|_| Tree::leaf(Counted(Rc::clone(&clones))))
            .collect(),
        0,
    );
    clones.set(0);
    let mut candidates = seq.shrinks();
    assert_eq!(clones.get(), 0);
    assert_eq!(candidates.next().unwrap().value().len(), 32);
    assert!(clones.get() < 128);
}

#[test]
fn debug_shows_the_value() {
    assert!(format!("{:?}", Tree::leaf(3u8)).contains("value: 3"));
}
