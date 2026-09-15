// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::expr`.

use crate::expr::{ColumnRef, Expr, Names};
use crate::tests::{column, table};
use crate::value::Value;

/// One table to render column names against.
fn tables() -> Vec<crate::schema::Table> {
    vec![table("t0", 1)]
}

#[test]
fn a_column_is_qualified_for_a_query_and_bare_for_an_index() {
    let tables = tables();
    let expr = column(0, 1);
    assert_eq!(expr.render(&tables, Names::Qualified), "t0.c1");
    assert_eq!(expr.render(&tables, Names::Bare), "c1");
}

#[test]
fn a_column_that_is_not_there_renders_as_null_rather_than_as_nothing() {
    let tables = tables();
    let missing_table = Expr::Column(ColumnRef {
        table: 9,
        column: 0,
    });
    let missing_column = Expr::Column(ColumnRef {
        table: 0,
        column: 9,
    });
    assert_eq!(missing_table.render(&tables, Names::Qualified), "NULL");
    assert_eq!(missing_column.render(&tables, Names::Bare), "NULL");
}

#[test]
fn every_node_is_parenthesized_when_it_is_written() {
    let tables = tables();
    let rendered = |expr: &Expr| expr.render(&tables, Names::Qualified);
    let one = Expr::Literal(Value::Int(1));
    assert_eq!(
        rendered(&Expr::Binary {
            op: "AND",
            left: Box::new(one.clone()),
            right: Box::new(column(0, 0)),
        }),
        "(1 AND t0.c0)"
    );
    assert_eq!(
        rendered(&Expr::Prefix {
            op: "NOT",
            operand: Box::new(one.clone()),
        }),
        "(NOT 1)"
    );
    assert_eq!(
        rendered(&Expr::Suffix {
            op: "IS NULL",
            operand: Box::new(one.clone()),
        }),
        "(1 IS NULL)"
    );
    assert_eq!(
        rendered(&Expr::Cast {
            operand: Box::new(one),
            ty: "TEXT",
        }),
        "CAST(1 AS TEXT)"
    );
}

#[test]
fn between_and_in_carry_their_negation_and_their_list() {
    let tables = tables();
    let value = || Box::new(column(0, 0));
    let between = Expr::Between {
        value: value(),
        low: Box::new(Expr::Literal(Value::Int(1))),
        high: Box::new(Expr::Literal(Value::Int(2))),
        negated: true,
    };
    assert_eq!(
        between.render(&tables, Names::Qualified),
        "(t0.c0 NOT BETWEEN 1 AND 2)"
    );
    let inside = Expr::In {
        value: value(),
        list: vec![Expr::Literal(Value::Int(1)), Expr::Literal(Value::Null)],
        negated: false,
    };
    assert_eq!(
        inside.render(&tables, Names::Qualified),
        "(t0.c0 IN (1, NULL))"
    );
}

#[test]
fn a_call_writes_its_arguments_in_order() {
    let tables = tables();
    let call = Expr::Call {
        name: "coalesce",
        args: vec![column(0, 0), Expr::Literal(Value::Int(0))],
    };
    assert_eq!(call.render(&tables, Names::Qualified), "coalesce(t0.c0, 0)");
    let none = Expr::Call {
        name: "abs",
        args: Vec::new(),
    };
    assert_eq!(none.render(&tables, Names::Qualified), "abs()");
}

#[test]
fn children_are_listed_in_the_order_they_are_written() {
    let leaf = Expr::Literal(Value::Int(1));
    assert!(leaf.children().is_empty());
    let binary = Expr::Binary {
        op: "+",
        left: Box::new(leaf.clone()),
        right: Box::new(column(0, 0)),
    };
    assert_eq!(binary.children().len(), 2);
    let inside = Expr::In {
        value: Box::new(leaf.clone()),
        list: vec![leaf.clone(), leaf.clone()],
        negated: false,
    };
    assert_eq!(inside.children().len(), 3);
    let between = Expr::Between {
        value: Box::new(leaf.clone()),
        low: Box::new(leaf.clone()),
        high: Box::new(leaf),
        negated: false,
    };
    assert_eq!(between.children().len(), 3);
}

#[test]
fn a_child_is_replaced_by_position_and_an_unknown_position_changes_nothing() {
    let one = Expr::Literal(Value::Int(1));
    let two = Expr::Literal(Value::Int(2));
    let binary = Expr::Binary {
        op: "+",
        left: Box::new(one.clone()),
        right: Box::new(one.clone()),
    };
    let tables = tables();
    assert_eq!(
        binary
            .with_child(1, two.clone())
            .render(&tables, Names::Qualified),
        "(1 + 2)"
    );
    assert_eq!(binary.with_child(5, two.clone()), binary);
    assert_eq!(one.with_child(0, two.clone()), one);

    let inside = Expr::In {
        value: Box::new(one.clone()),
        list: vec![one.clone()],
        negated: false,
    };
    assert_eq!(
        inside
            .with_child(1, two.clone())
            .render(&tables, Names::Qualified),
        "(1 IN (2))"
    );
    let call = Expr::Call {
        name: "abs",
        args: vec![one.clone()],
    };
    assert_eq!(
        call.with_child(0, two.clone())
            .render(&tables, Names::Qualified),
        "abs(2)"
    );
    let between = Expr::Between {
        value: Box::new(one.clone()),
        low: Box::new(one.clone()),
        high: Box::new(one.clone()),
        negated: false,
    };
    assert_eq!(
        between
            .with_child(2, two.clone())
            .render(&tables, Names::Qualified),
        "(1 BETWEEN 1 AND 2)"
    );
    let cast = Expr::Cast {
        operand: Box::new(one),
        ty: "TEXT",
    };
    assert_eq!(
        cast.with_child(0, two).render(&tables, Names::Qualified),
        "CAST(2 AS TEXT)"
    );
}

#[test]
fn the_size_of_a_tree_counts_every_node() {
    let leaf = Expr::Literal(Value::Int(1));
    assert_eq!(leaf.size(), 1);
    let tree = Expr::Binary {
        op: "+",
        left: Box::new(leaf.clone()),
        right: Box::new(Expr::Prefix {
            op: "-",
            operand: Box::new(leaf),
        }),
    };
    assert_eq!(tree.size(), 4);
}

#[test]
fn only_a_tree_that_names_a_column_may_be_indexed() {
    let leaf = Expr::Literal(Value::Int(1));
    assert!(!leaf.mentions_column());
    assert!(column(0, 0).mentions_column());
    assert!(
        Expr::Call {
            name: "abs",
            args: vec![column(0, 0)],
        }
        .mentions_column()
    );
}

#[test]
fn every_candidate_is_smaller_than_the_tree_it_came_from() {
    let tree = Expr::Binary {
        op: "AND",
        left: Box::new(column(0, 0)),
        right: Box::new(Expr::Prefix {
            op: "NOT",
            operand: Box::new(Expr::Literal(Value::Int(1))),
        }),
    };
    let candidates = tree.candidates();
    assert!(!candidates.is_empty());
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.size() < tree.size())
    );
    assert!(candidates.contains(&column(0, 0)));
    assert!(candidates.contains(&Expr::Literal(Value::Null)));
    // A literal is already smallest, so it offers nothing.
    assert!(Expr::Literal(Value::Int(1)).candidates().is_empty());
}
