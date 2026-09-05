// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::pool`.

use std::path::PathBuf;

use crate::pool::{Pool, Verdict};
use crate::rng::Rng;

#[test]
fn an_empty_pool_holds_nothing_and_draws_nothing() {
    let pool = Pool::new();
    assert!(pool.is_empty());
    assert_eq!(pool.len(), 0);
    assert_eq!(pool.bytes(), 0);
    assert_eq!(pool.covered(), 0);
    assert_eq!(pool.get(0).map(|input| input.live), None);
    assert_eq!(pool.choose(&mut Rng::new(1)), None);
    assert!(pool.all().is_empty());
}

#[test]
fn an_input_that_reaches_something_new_is_kept() {
    let mut pool = Pool::default();
    assert_eq!(pool.offer(b"abc", &[1, 2, 3], true), Verdict::New);
    assert_eq!(pool.len(), 1);
    assert_eq!(pool.bytes(), 3);
    assert_eq!(pool.covered(), 3);
    assert_eq!(pool.offer(b"abcd", &[1, 2, 3], true), Verdict::Nothing);
    assert_eq!(pool.len(), 1);
}

#[test]
fn a_shorter_input_takes_a_feature_over_and_may_empty_the_one_that_held_it() {
    let mut pool = Pool::new();
    assert_eq!(pool.offer(b"longlong", &[10, 11], true), Verdict::New);
    assert_eq!(pool.offer(b"sh", &[10, 11], true), Verdict::Reduced);
    assert_eq!(pool.len(), 1, "the longer input owns nothing and is gone");
    assert_eq!(pool.bytes(), 2);
    assert_eq!(pool.covered(), 2, "no feature was added, only moved");
    assert!(pool.get(0).is_some_and(|input| !input.live));
    assert!(pool.get(1).is_some_and(|input| input.live));
}

#[test]
fn a_shorter_input_is_refused_when_the_run_is_not_shrinking() {
    let mut pool = Pool::new();
    assert_eq!(pool.offer(b"longlong", &[10], false), Verdict::New);
    assert_eq!(pool.offer(b"s", &[10], false), Verdict::Nothing);
    assert_eq!(pool.len(), 1);
}

#[test]
fn an_input_that_adds_one_feature_to_many_old_ones_is_new() {
    let mut pool = Pool::new();
    assert_eq!(pool.offer(b"a", &[1], true), Verdict::New);
    assert_eq!(pool.offer(b"bb", &[1, 2], true), Verdict::New);
    assert_eq!(pool.len(), 2);
    assert_eq!(pool.covered(), 2);
}

#[test]
fn what_a_pool_covers_is_asked_of_it_by_size() {
    let mut pool = Pool::new();
    assert!(!pool.covers(5, 100));
    let _ = pool.offer(b"abcd", &[5], true);
    assert!(pool.covers(5, 4));
    assert!(pool.covers(5, 9));
    assert!(!pool.covers(5, 3));
}

#[test]
fn a_draw_only_ever_lands_on_an_input_that_owns_something() {
    let mut pool = Pool::new();
    let _ = pool.offer(b"aaaa", &[1], true);
    let _ = pool.offer(b"a", &[1], true);
    let _ = pool.offer(b"bbb", &[2], true);
    let mut rng = Rng::new(3);
    for _ in 0..500 {
        let index = pool.choose(&mut rng).unwrap();
        assert!(pool.get(index).is_some_and(|input| input.live));
    }
}

#[test]
fn where_an_input_came_from_is_remembered() {
    let mut pool = Pool::new();
    let index = pool.next_index();
    let _ = pool.offer(b"x", &[1], true);
    pool.set_file(index, PathBuf::from("/tmp/x"));
    assert_eq!(
        pool.get(index).and_then(|input| input.file.clone()),
        Some(PathBuf::from("/tmp/x"))
    );
    pool.set_file(999, PathBuf::from("/tmp/nowhere"));
}

#[test]
fn a_feature_beyond_the_table_shares_a_slot_with_an_older_one() {
    let mut pool = Pool::new();
    assert_eq!(pool.offer(b"aa", &[7], true), Verdict::New);
    assert_eq!(pool.offer(b"bb", &[7 + (1 << 21)], true), Verdict::Nothing);
}
