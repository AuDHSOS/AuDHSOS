// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::schema`.

use crate::expr::Expr;
use crate::schema::{Affinity, Column, Index, Table};
use crate::tests::{column, table};
use crate::value::Value;

#[test]
fn a_table_without_a_declared_type_is_created_without_one() {
    let mut subject = table("t0", 0);
    if let Some(first) = subject.columns.get_mut(0) {
        first.affinity = Affinity::None;
    }
    assert_eq!(subject.create(), "CREATE TABLE t0(c0, c1 TEXT);");
    assert_eq!(Affinity::Numeric.keyword(), "NUMERIC");
    assert_eq!(Affinity::Blob.keyword(), "BLOB");
    assert_eq!(Affinity::Real.keyword(), "REAL");
}

#[test]
fn a_declared_collation_follows_the_type() {
    let subject = Table {
        name: "t0".to_owned(),
        columns: vec![Column {
            name: "c0".to_owned(),
            affinity: Affinity::Text,
            collation: Some("NOCASE"),
        }],
        rows: Vec::new(),
        indexes: Vec::new(),
    };
    assert_eq!(subject.create(), "CREATE TABLE t0(c0 TEXT COLLATE NOCASE);");
}

#[test]
fn an_empty_table_has_no_insert_and_a_filled_one_has_a_single_one() {
    assert_eq!(table("t0", 0).insert(), None);
    assert_eq!(
        table("t0", 2).insert(),
        Some("INSERT INTO t0 VALUES (0, 'r0'), (1, 'r1');".to_owned())
    );
}

#[test]
fn an_index_carries_its_terms_unqualified_and_its_filter() {
    let mut subject = table("t0", 1);
    let tables = vec![subject.clone()];
    subject.indexes = vec![
        Index {
            name: "i0".to_owned(),
            unique: false,
            terms: vec![column(0, 0), column(0, 1)],
            filter: None,
        },
        Index {
            name: "i1".to_owned(),
            unique: true,
            terms: vec![Expr::Call {
                name: "abs",
                args: vec![column(0, 0)],
            }],
            filter: Some(Expr::Binary {
                op: ">",
                left: Box::new(column(0, 0)),
                right: Box::new(Expr::Literal(Value::Int(0))),
            }),
        },
    ];
    assert_eq!(
        subject.index_statements(&tables),
        vec![
            "CREATE INDEX i0 ON t0(c0, c1);".to_owned(),
            "CREATE UNIQUE INDEX i1 ON t0(abs(c0)) WHERE (c0 > 0);".to_owned(),
        ]
    );
}
