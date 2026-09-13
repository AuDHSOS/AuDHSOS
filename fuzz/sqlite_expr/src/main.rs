// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The expression parser against arbitrary bytes: no input may panic, no
//! input may run the stack out, and a tree that comes back is a tree.
//!
//! A parser is where a stack overflow lives, because the input decides how
//! deep the walk goes. The bound is in the parser and this target is what
//! holds it: a statement of ten thousand brackets is refused rather than
//! followed.

use db_sqlite::ast::{Arena, ExprId, Node};
use db_sqlite::parse::{Expected, expression};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    match expression(bytes) {
        Err(error) => {
            assert!(error.at <= bytes.len(), "a refusal past the end of the input");
            assert!(
                error.at.saturating_add(error.len) <= bytes.len(),
                "a refusal longer than the input"
            );
        }
        Ok((arena, root)) => {
            assert!(!arena.is_empty(), "a tree of no nodes");
            // Every node the tree names is a node the arena holds, and
            // every child of it is one the arena holds as well.
            walk(&arena, root, 0);
        }
    }
});

/// Reads every node reachable from `id`, and holds that the tree is one:
/// a child is always a node the arena has.
fn walk(arena: &Arena, id: ExprId, depth: u32) {
    // The parser's own bound is two hundred; a tree deeper than twice that
    // would mean the bound did not hold.
    assert!(depth < 512, "a tree deeper than the parser walks");
    let node = arena.node(id).expect("a node the arena does not hold");
    let mut child = |id| walk(arena, id, depth.saturating_add(1));
    match node {
        Node::Literal(_) | Node::Column { .. } | Node::Variable(_) => {}
        Node::Unary { operand, .. } => child(operand),
        Node::Binary { left, right, .. } => {
            child(left);
            child(right);
        }
        Node::Between {
            value, low, high, ..
        } => {
            child(value);
            child(low);
            child(high);
        }
        Node::InList { value, list, .. } => {
            child(value);
            for item in arena.children(list) {
                child(*item);
            }
        }
        Node::Like {
            value,
            pattern,
            escape,
            ..
        } => {
            child(value);
            child(pattern);
            if let Some(escape) = escape {
                child(escape);
            }
        }
        Node::Cast { value, .. } | Node::Collate { value, .. } => child(value),
        Node::Call { args, .. } => {
            for arg in arena.children(args) {
                child(*arg);
            }
        }
        Node::Case {
            operand,
            branches,
            otherwise,
        } => {
            for id in operand.into_iter().chain(otherwise) {
                child(id);
            }
            for branch in arena.children(branches) {
                child(*branch);
            }
        }
        Node::Row(items) => {
            for item in arena.children(items) {
                child(*item);
            }
        }
    }
    let _ = Expected::Depth;
}
