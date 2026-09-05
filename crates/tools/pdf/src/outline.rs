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
/// Everything else — the neighbours, the children, the count of what lies
/// below an entry — is a question about that one table, and is answered by
/// asking it rather than by writing back into the nodes.
#[must_use]
pub fn tree(entries: &[Entry]) -> Vec<Node> {
    let parents = parents(entries);
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let parent = parent_of(&parents, index);
            let children = children(&parents, index);
            Node {
                entry: entry.clone(),
                parent,
                previous: (0..index)
                    .rev()
                    .find(|before| parent_of(&parents, *before) == parent),
                next: (index.saturating_add(1)..parents.len())
                    .find(|after| parent_of(&parents, *after) == parent),
                first: children.first().copied(),
                last: children.last().copied(),
                descendants: (0..parents.len())
                    .filter(|other| descends_from(&parents, *other, index))
                    .count(),
            }
        })
        .collect()
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

/// The parent of one entry.
fn parent_of(parents: &[Option<usize>], index: usize) -> Option<usize> {
    parents.get(index).copied().flatten()
}

/// The entries whose parent is `index`, in order.
fn children(parents: &[Option<usize>], index: usize) -> Vec<usize> {
    parents
        .iter()
        .enumerate()
        .filter(|(_, parent)| **parent == Some(index))
        .map(|(child, _)| child)
        .collect()
}

/// Whether `index` lies below `ancestor`, at any depth.
fn descends_from(parents: &[Option<usize>], index: usize, ancestor: usize) -> bool {
    let mut walker = parent_of(parents, index);
    while let Some(node) = walker {
        if node == ancestor {
            return true;
        }
        walker = parent_of(parents, node);
    }
    false
}
