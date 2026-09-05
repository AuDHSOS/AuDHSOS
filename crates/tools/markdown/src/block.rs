// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a document is made of.

use crate::inline::Inline;

/// How a table column is aligned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// The default, and what a column with no marker gets.
    Left,
    /// Centred, written `:---:`.
    Center,
    /// Right, written `---:`.
    Right,
}

/// One block of a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// A heading, `level` between one and six.
    Heading {
        /// How deep the heading is.
        level: usize,
        /// What it says.
        content: Vec<Inline>,
    },
    /// A run of prose.
    Paragraph(Vec<Inline>),
    /// A fenced or indented code block, kept line by line and never
    /// reflowed.
    Code {
        /// The word after the opening fence, if there was one.
        language: Option<String>,
        /// The lines, without their trailing newline.
        lines: Vec<String>,
    },
    /// A quotation, which holds blocks of its own.
    Quote(Vec<Block>),
    /// A list. Every item holds blocks, so a list can hold a paragraph, a
    /// code block, or another list.
    List {
        /// Whether the items are numbered.
        ordered: bool,
        /// The number the first item carries.
        start: u64,
        /// Whether the list was written without a blank line inside it.
        /// A tight list is set with its items close together; a loose one
        /// is set with the space between paragraphs, which is what its
        /// author asked for by leaving the blank line.
        tight: bool,
        /// The items.
        items: Vec<Vec<Block>>,
    },
    /// A table with a header row.
    Table {
        /// The alignment of every column.
        alignments: Vec<Align>,
        /// The header row.
        header: Vec<Vec<Inline>>,
        /// The body rows.
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// A horizontal rule.
    Rule,
}

impl Block {
    /// The plain text of a heading, for an outline entry or a title.
    #[must_use]
    pub fn heading_text(&self) -> Option<String> {
        match self {
            Block::Heading { content, .. } => Some(crate::inline::plain(content)),
            _ => None,
        }
    }
}
