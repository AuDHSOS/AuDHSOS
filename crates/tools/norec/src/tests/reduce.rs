// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::reduce`.

use crate::case::{CountStyle, JoinKind};
use crate::expr::Expr;
use crate::reduce::reduce;
use crate::schema::Index;
use crate::tests::{Fake, answer, case, column, joined};
use crate::value::Value;

/// An engine that disagrees over every case it is given.
fn always() -> impl crate::engine::Engine {
    Fake {
        answer: |_: &str| answer(&["1", "2"], "1"),
    }
}

/// An engine that disagrees only while the script holds `needle`.
fn only_with(needle: &'static str) -> impl crate::engine::Engine {
    Fake {
        answer: move |script: &str| {
            if script.contains(needle) {
                answer(&["1", "2"], "1")
            } else {
                answer(&["1"], "1")
            }
        },
    }
}

#[test]
fn a_case_that_always_disagrees_is_reduced_to_its_smallest_form() {
    let mut engine = always();
    let mut subject = joined(case(4), JoinKind::Left);
    subject.analyze = true;
    let reduced = reduce(&mut engine, &subject, CountStyle::Rows, 400).unwrap();
    assert!(reduced.size() < subject.size());
    assert!(reduced.source.joins.is_empty());
    assert!(!reduced.analyze);
    assert!(reduced.tables.iter().all(|table| table.rows.is_empty()));
}

#[test]
fn what_the_disagreement_needs_survives_the_reduction() {
    let mut engine = only_with("t0.c1");
    let mut subject = case(4);
    subject.predicate = Expr::Binary {
        op: "AND",
        left: Box::new(Expr::Binary {
            op: "=",
            left: Box::new(column(0, 0)),
            right: Box::new(Expr::Literal(Value::Int(1))),
        }),
        right: Box::new(Expr::Suffix {
            op: "IS NOT NULL",
            operand: Box::new(column(0, 1)),
        }),
    };
    let reduced = reduce(&mut engine, &subject, CountStyle::Rows, 400).unwrap();
    assert!(reduced.render(&reduced.predicate).contains("t0.c1"));
    assert!(reduced.size() < subject.size());
}

#[test]
fn an_index_the_disagreement_needs_is_kept_and_another_is_dropped() {
    let mut engine = only_with("CREATE INDEX i1");
    let mut subject = case(2);
    if let Some(table) = subject.tables.get_mut(0) {
        table.indexes = vec![
            Index {
                name: "i0".to_owned(),
                unique: false,
                terms: vec![column(0, 0)],
                filter: None,
            },
            Index {
                name: "i1".to_owned(),
                unique: false,
                terms: vec![column(0, 1)],
                filter: None,
            },
        ];
    }
    let reduced = reduce(&mut engine, &subject, CountStyle::Rows, 400).unwrap();
    let names: Vec<&str> = reduced
        .tables
        .iter()
        .flat_map(|table| table.indexes.iter().map(|index| index.name.as_str()))
        .collect();
    assert_eq!(names, ["i1"]);
}

#[test]
fn a_case_that_stops_disagreeing_is_left_as_it_was() {
    let mut engine = Fake {
        answer: |_: &str| answer(&["1"], "1"),
    };
    let subject = case(3);
    let reduced = reduce(&mut engine, &subject, CountStyle::Rows, 400).unwrap();
    assert_eq!(reduced.size(), subject.size());
}

#[test]
fn reduction_stops_when_its_budget_is_spent() {
    let mut engine = always();
    let subject = case(6);
    let reduced = reduce(&mut engine, &subject, CountStyle::Rows, 1).unwrap();
    // One accepted candidate, and then nothing left to spend.
    assert!(reduced.size() < subject.size());
    let none = reduce(&mut engine, &subject, CountStyle::Rows, 0).unwrap();
    assert_eq!(none.size(), subject.size());
}
