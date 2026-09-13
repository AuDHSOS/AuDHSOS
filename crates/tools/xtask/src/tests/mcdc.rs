// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The reader of the typed tree.

use core::fmt::Write as _;
use std::path::Path;

use crate::mcdc::{Place, boolean_bitwise, read, repeated_conditions};

/// A typed tree of one binary operator over operands of `ty`.
fn tree(operator: &str, ty: &str) -> String {
    format!(
        "Expr {{\n    ty: {ty}\n    span: src/a.rs:1:2: 1:9 (#0)\n    kind: \n        Binary {{\n            op: {operator}\n            lhs:\n                Expr {{\n                    ty: {ty}\n                }}\n        }}\n}}\n"
    )
}

#[test]
fn a_bitwise_operator_over_booleans_is_a_violation() {
    for operator in ["BitAnd", "BitOr", "BitXor"] {
        let found = boolean_bitwise(&tree(operator, "bool"));
        assert_eq!(found.len(), 1, "{operator}");
        assert!(
            found[0].starts_with("src/a.rs:1:2: 1:9 (#0): "),
            "{operator}"
        );
        assert!(found[0].contains(operator), "{operator}");
    }
}

#[test]
fn a_bitwise_operator_over_numbers_is_not() {
    for ty in ["u8", "u32", "u64", "i64", "u128"] {
        assert!(boolean_bitwise(&tree("BitAnd", ty)).is_empty(), "{ty}");
    }
}

#[test]
fn a_short_circuit_operator_is_not_a_bitwise_one() {
    let tree = "Expr {\n    ty: bool\n    kind: \n        LogicalOp {\n            op: And\n        }\n}\n";
    assert!(boolean_bitwise(tree).is_empty());
}

#[test]
fn an_operator_whose_tree_names_no_operand_is_passed_over() {
    let tree = "Expr {\n    ty: bool\n    kind: \n        Binary {\n            op: BitAnd\n";
    assert!(boolean_bitwise(tree).is_empty());
}

#[test]
fn an_operator_the_tree_gives_no_span_is_still_named() {
    let tree =
        "Binary {\n    op: BitAnd\n    lhs:\n        Expr {\n            ty: bool\n        }\n}\n";
    let found = boolean_bitwise(tree);
    assert_eq!(found.len(), 1);
    assert!(found[0].starts_with("an unnamed span: "));
}

/// A typed tree of one decision over `conditions`, each named by its own
/// span, written at the indentation the pretty printer uses.
fn decision(conditions: &[&str]) -> String {
    fn operand(depth: usize, text: &str) -> String {
        let pad = " ".repeat(depth.saturating_mul(4));
        format!("{pad}Expr {{\n{pad}    ty: bool\n{pad}    span: {text}\n{pad}    kind: \n")
    }
    fn logical(depth: usize, conditions: &[&str]) -> String {
        let pad = " ".repeat(depth.saturating_mul(4));
        let Some((last, rest)) = conditions.split_last() else {
            return String::new();
        };
        if rest.is_empty() {
            return format!("{pad}VarRef {{\n{pad}    id: one\n{pad}}}\n");
        }
        let mut out = format!("{pad}LogicalOp {{\n{pad}    op: And\n{pad}    lhs:\n");
        let left = if rest.len() == 1 { rest[0] } else { "nested" };
        out.push_str(&operand(depth.saturating_add(2), left));
        out.push_str(&logical(depth.saturating_add(4), rest));
        let _ = writeln!(out, "{pad}    rhs:");
        out.push_str(&operand(depth.saturating_add(2), last));
        out.push_str(&logical(depth.saturating_add(4), &[last]));
        let _ = writeln!(out, "{pad}}}");
        out
    }
    let mut out = operand(0, "whole");
    out.push_str(&logical(2, conditions));
    out
}

#[test]
fn a_decision_counts_one_condition_per_operand() {
    // The spans point into this file, because the reader answers the
    // source they name: the first two lines begin `// SPDX` and
    // `// Copyright`, so columns 1 to 8 differ and columns 1 to 3 agree.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let here = "src/tests/mcdc.rs";
    let tree = decision(&[
        &format!("{here}:1:1: 1:8 (#0)"),
        &format!("{here}:2:1: 2:8 (#0)"),
    ]);
    let (conditions, violations) = repeated_conditions(root, &read(&tree)).unwrap();
    assert_eq!(conditions, 2);
    assert!(violations.is_empty(), "{violations:?}");
    let tree = decision(&[
        &format!("{here}:1:1: 1:3 (#0)"),
        &format!("{here}:2:1: 2:3 (#0)"),
    ]);
    let (conditions, violations) = repeated_conditions(root, &read(&tree)).unwrap();
    assert_eq!(conditions, 2);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].contains("`//` is named twice by one decision"));
}

#[test]
fn a_decision_of_three_conditions_reads_all_three() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let here = "src/tests/mcdc.rs";
    let tree = decision(&[
        &format!("{here}:1:1: 1:8 (#0)"),
        &format!("{here}:2:1: 2:8 (#0)"),
        &format!("{here}:4:1: 4:8 (#0)"),
    ]);
    let (conditions, violations) = repeated_conditions(root, &read(&tree)).unwrap();
    assert_eq!(conditions, 3);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn a_condition_that_runs_over_two_lines_is_read_as_one() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let here = "src/tests/mcdc.rs";
    let tree = decision(&[
        &format!("{here}:1:1: 2:8 (#0)"),
        &format!("{here}:4:1: 4:8 (#0)"),
    ]);
    let (conditions, violations) = repeated_conditions(root, &read(&tree)).unwrap();
    assert_eq!(conditions, 2);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn a_span_the_compiler_wrote_from_a_macro_is_passed_over() {
    assert!(Place::read("a.rs:1:1: 1:2 (#288)").is_none());
    assert!(Place::read("a.rs:1:1 (#0)").is_none());
    assert!(Place::read("1:2: 3:4 (#0)").is_none());
    let place = Place::read("crates/db/sqlite/src/tree.rs:12:3: 12:40 (#0)").unwrap();
    assert_eq!(place.path, "crates/db/sqlite/src/tree.rs");
    assert_eq!(place.from, (12, 3));
    assert_eq!(place.to, (12, 40));
}

#[test]
fn the_tree_is_read_without_its_empty_lines() {
    let lines = read("a\n\n    b\n   \n        c\n");
    assert_eq!(lines.len(), 3);
}
