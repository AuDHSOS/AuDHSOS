// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The outline tree.

use crate::outline::{Entry, tree};
use crate::units::pt;

fn entry(level: usize, title: &str) -> Entry {
    Entry {
        level,
        title: title.to_owned(),
        page: 0,
        top: pt(700),
    }
}

#[test]
fn an_empty_list_makes_an_empty_tree() {
    assert!(tree(&[]).is_empty());
}

#[test]
fn siblings_are_linked_in_both_directions() {
    let nodes = tree(&[entry(0, "one"), entry(0, "two"), entry(0, "three")]);
    assert_eq!(nodes.first().and_then(|n| n.next), Some(1));
    assert_eq!(nodes.get(1).and_then(|n| n.previous), Some(0));
    assert_eq!(nodes.get(1).and_then(|n| n.next), Some(2));
    assert_eq!(nodes.get(2).and_then(|n| n.next), None);
    assert!(nodes.iter().all(|node| node.parent.is_none()));
}

#[test]
fn a_child_names_its_parent_and_the_parent_its_children() {
    let nodes = tree(&[
        entry(0, "chapter"),
        entry(1, "first"),
        entry(1, "second"),
        entry(0, "next chapter"),
    ]);
    assert_eq!(nodes.get(1).and_then(|n| n.parent), Some(0));
    assert_eq!(nodes.get(2).and_then(|n| n.parent), Some(0));
    assert_eq!(nodes.first().and_then(|n| n.first), Some(1));
    assert_eq!(nodes.first().and_then(|n| n.last), Some(2));
    assert_eq!(nodes.first().map(|n| n.descendants), Some(2));
    assert_eq!(nodes.get(3).and_then(|n| n.parent), None);
    assert_eq!(nodes.first().and_then(|n| n.next), Some(3));
}

#[test]
fn a_descendant_counts_at_every_depth_above_it() {
    let nodes = tree(&[entry(0, "a"), entry(1, "b"), entry(2, "c"), entry(3, "d")]);
    assert_eq!(nodes.first().map(|n| n.descendants), Some(3));
    assert_eq!(nodes.get(1).map(|n| n.descendants), Some(2));
    assert_eq!(nodes.get(2).map(|n| n.descendants), Some(1));
    assert_eq!(nodes.get(3).map(|n| n.descendants), Some(0));
}

#[test]
fn closing_a_level_reopens_the_one_above() {
    let nodes = tree(&[
        entry(0, "a"),
        entry(1, "a.1"),
        entry(2, "a.1.1"),
        entry(1, "a.2"),
    ]);
    assert_eq!(nodes.get(3).and_then(|n| n.parent), Some(0));
    assert_eq!(nodes.get(3).and_then(|n| n.previous), Some(1));
    assert_eq!(nodes.get(1).and_then(|n| n.next), Some(3));
}

#[test]
fn every_link_points_at_a_node_that_exists() {
    let nodes = tree(&[
        entry(0, "a"),
        entry(1, "b"),
        entry(1, "c"),
        entry(0, "d"),
        entry(2, "e"),
    ]);
    for node in &nodes {
        for link in [node.parent, node.previous, node.next, node.first, node.last]
            .into_iter()
            .flatten()
        {
            assert!(link < nodes.len(), "link {link} is outside the tree");
        }
    }
}
