// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The reader of the typed tree.

use crate::mcdc::boolean_bitwise;

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
