// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::report`.

use crate::case::CountStyle;
use crate::oracle::Verdict;
use crate::report::{reproducer, write};
use crate::tests::{Scratch, case};

#[test]
fn the_reproducer_holds_the_database_and_both_queries() {
    let verdict = Verdict::Mismatch {
        optimized: 2,
        unoptimized: 1,
    };
    let text = reproducer(&case(2), CountStyle::Rows, &verdict, 42);
    assert!(text.contains("-- seed 42"));
    assert!(text.contains("selected 2 row(s)"));
    assert!(text.contains("true 1 time(s)"));
    assert!(text.contains("CREATE TABLE t0(c0 INTEGER, c1 TEXT);"));
    assert!(text.contains("SELECT * FROM t0 WHERE (t0.c0 = 1);"));
    assert!(text.contains("SELECT SUM(count) FROM"));
}

#[test]
fn a_verdict_that_is_not_a_disagreement_leaves_the_counts_out() {
    let text = reproducer(&case(1), CountStyle::Count, &Verdict::Agree(1), 1);
    assert!(!text.contains("selected"));
    assert!(text.contains("SELECT COUNT(*) FROM t0 WHERE (t0.c0 = 1);"));
}

#[test]
fn writing_creates_the_directory_and_names_the_file_after_the_seed() {
    let scratch = Scratch::new();
    let nested = scratch.0.join("reports");
    let path = write(&nested, 7, "-- text\n").unwrap();
    assert!(path.ends_with("norec-7.sql"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "-- text\n");
}

#[test]
fn a_directory_that_cannot_be_created_is_an_error() {
    let scratch = Scratch::new();
    let file = scratch.0.join("file");
    std::fs::create_dir_all(&scratch.0).unwrap();
    std::fs::write(&file, "").unwrap();
    assert!(write(&file.join("under"), 1, "").is_err());
}
