// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::case`.

use crate::case::{Case, CountStyle, JoinKind, Source};
use crate::tests::{case, joined};

#[test]
fn one_table_is_the_whole_source_clause() {
    assert_eq!(case(1).source_clause(), "t0");
}

#[test]
fn each_join_writes_the_form_it_is() {
    assert_eq!(joined(case(1), JoinKind::Comma).source_clause(), "t0, t1");
    assert_eq!(
        joined(case(1), JoinKind::Inner).source_clause(),
        "t0 JOIN t1 ON (t0.c0 = t1.c0)"
    );
    assert_eq!(
        joined(case(1), JoinKind::Left).source_clause(),
        "t0 LEFT JOIN t1 ON (t0.c0 = t1.c0)"
    );
}

#[test]
fn a_join_without_a_condition_still_writes_one() {
    let mut subject = joined(case(1), JoinKind::Inner);
    if let Some(join) = subject.source.joins.get_mut(0) {
        join.on = None;
    }
    assert_eq!(subject.source_clause(), "t0 JOIN t1 ON 1");
}

#[test]
fn a_table_the_case_does_not_have_does_not_stop_the_clause() {
    let subject = Case {
        source: Source {
            first: 7,
            joins: Vec::new(),
        },
        ..case(1)
    };
    assert_eq!(subject.source_clause(), "sqlite_schema");
}

#[test]
fn the_optimized_query_filters_and_the_unoptimized_one_sums() {
    let subject = case(2);
    assert_eq!(
        subject.optimized(CountStyle::Rows),
        "SELECT * FROM t0 WHERE (t0.c0 = 1);"
    );
    assert_eq!(
        subject.optimized(CountStyle::Count),
        "SELECT COUNT(*) FROM t0 WHERE (t0.c0 = 1);"
    );
    assert_eq!(
        subject.unoptimized(),
        "SELECT SUM(count) FROM (SELECT ((t0.c0 = 1)) IS TRUE AS count FROM t0);"
    );
}

#[test]
fn the_setup_creates_fills_and_indexes_every_table_in_order() {
    let mut subject = joined(case(1), JoinKind::Comma);
    subject.analyze = true;
    let setup = subject.setup();
    assert_eq!(setup.len(), 5);
    assert!(
        setup
            .first()
            .is_some_and(|line| line.starts_with("CREATE TABLE t0"))
    );
    assert!(
        setup
            .get(1)
            .is_some_and(|line| line.starts_with("INSERT INTO t0"))
    );
    assert_eq!(setup.last().map(String::as_str), Some("ANALYZE;"));
}

#[test]
fn the_size_counts_rows_indexes_joins_and_the_predicate() {
    let one = case(3);
    // Three rows and three predicate nodes.
    assert_eq!(one.size(), 6);
    let mut two = joined(one, JoinKind::Comma);
    two.analyze = true;
    // One row of t1, the join and the analyze on top.
    assert_eq!(two.size(), 9);
}
