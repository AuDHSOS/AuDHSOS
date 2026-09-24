// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The outline, from a flat list of headings to the tree a viewer wants.
//!
//! A document is written as a sequence of headings, each with a level; the
//! outline of a PDF is a doubly linked tree, where every entry names its
//! parent, its neighbours, its first and last child, and how many
//! descendants it has. The conversion happens once, here, so that neither
//! the layout nor the writer has to think about it.

use crate::units::Mils;

/// One heading, as the layout hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Depth, zero for a top-level heading.
    pub level: usize,
    /// What the outline shows.
    pub title: String,
    /// The page the heading is on.
    pub page: usize,
    /// The height on that page the view is scrolled to.
    pub top: Mils,
}

/// One entry, with the links a PDF outline item needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The heading itself.
    pub entry: Entry,
    /// The enclosing entry, if any.
    pub parent: Option<usize>,
    /// The entry before this one under the same parent.
    pub previous: Option<usize>,
    /// The entry after this one under the same parent.
    pub next: Option<usize>,
    /// The first child.
    pub first: Option<usize>,
    /// The last child.
    pub last: Option<usize>,
    /// How many entries are below this one, at any depth.
    pub descendants: usize,
}

/// Turns the flat list into the tree.
///
/// The parents are found first, in one pass: a stack holds the open entry
/// of every level, and an entry hangs under whatever is open above it.
/// A forward pass links siblings and children. A backward pass counts
/// descendants. The work is O(n) for n entries.
#[must_use]
pub fn tree(entries: &[Entry]) -> Vec<Node> {
    let parents = parents(entries);
    let mut nodes: Vec<Node> = entries
        .iter()
        .zip(parents.iter().copied())
        .map(|(entry, parent)| Node {
            entry: entry.clone(),
            parent,
            previous: None,
            next: None,
            first: None,
            last: None,
            descendants: 0,
        })
        .collect();
    let mut last_root = None;
    for (index, parent) in parents.iter().copied().enumerate() {
        let previous = if let Some(parent) = parent {
            let Some(parent_node) = nodes.get_mut(parent) else {
                continue;
            };
            let previous = parent_node.last;
            if parent_node.first.is_none() {
                parent_node.first = Some(index);
            }
            parent_node.last = Some(index);
            previous
        } else {
            let previous = last_root;
            last_root = Some(index);
            previous
        };
        if let Some(node) = nodes.get_mut(index) {
            node.previous = previous;
        }
        if let Some(previous) = previous
            && let Some(node) = nodes.get_mut(previous)
        {
            node.next = Some(index);
        }
    }
    for (index, parent) in parents.iter().copied().enumerate().rev() {
        if let Some(parent) = parent {
            let descendants = nodes
                .get(index)
                .map_or(0, |node| node.descendants.saturating_add(1));
            if let Some(parent_node) = nodes.get_mut(parent) {
                parent_node.descendants = parent_node.descendants.saturating_add(descendants);
            }
        }
    }
    nodes
}

/// The parent of every entry, by index.
fn parents(entries: &[Entry]) -> Vec<Option<usize>> {
    // The stack holds the open entry of every level: the entry at index
    // `level` is the one a following entry of `level + 1` belongs under.
    let mut open: Vec<usize> = Vec::new();
    let mut parents = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        open.truncate(entry.level);
        parents.push(open.last().copied());
        open.push(index);
    }
    parents
}
