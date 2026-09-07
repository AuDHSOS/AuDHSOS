// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The document of this repository, which is the input the parser has to
//! be right about. It is one file of seven megabytes, so it is read and
//! parsed once and everything that has to hold is asserted of the same
//! result.

use std::path::{Path, PathBuf};

use doc_markdown::{Block, plain};

use crate::{parse, title};

/// The workspace root, from the manifest directory of this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Every HTML document kept under `docs/`.
fn documents() -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(&root().join("docs"), &mut found);
    found.sort();
    found
}

fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, found);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "html")
        {
            found.push(path);
        }
    }
}

/// What is counted in a set of blocks, at every depth.
#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    /// Headings, by the level they carry.
    headings: Vec<usize>,
    /// Paragraphs.
    paragraphs: usize,
    /// Blocks of preformatted text.
    code: usize,
    /// Lists.
    lists: usize,
    /// Tables.
    tables: usize,
    /// Headings with nothing in them, which there must be none of.
    empty: usize,
}

impl Counts {
    fn of(blocks: &[Block]) -> Self {
        let mut counts = Self::default();
        counts.walk(blocks);
        counts
    }

    fn walk(&mut self, blocks: &[Block]) {
        for block in blocks {
            match block {
                Block::Heading { level, content } => {
                    self.headings.push(*level);
                    if plain(content).trim().is_empty() {
                        self.empty = self.empty.saturating_add(1);
                    }
                }
                Block::Paragraph(_) => self.paragraphs = self.paragraphs.saturating_add(1),
                Block::Code { .. } => self.code = self.code.saturating_add(1),
                Block::Quote(inner) => self.walk(inner),
                Block::List { items, .. } => {
                    self.lists = self.lists.saturating_add(1);
                    for item in items {
                        self.walk(item);
                    }
                }
                Block::Table { .. } => self.tables = self.tables.saturating_add(1),
                Block::Rule => {}
            }
        }
    }
}

#[test]
fn the_specification_comes_out_as_a_document() {
    let Some(path) = documents().first().cloned() else {
        return;
    };
    let Ok(html) = std::fs::read_to_string(&path) else {
        return;
    };
    let name = path.display().to_string();

    assert!(
        title(&html).is_some_and(|title| title.contains("ECMAScript")),
        "{name} has no title of its own"
    );

    let blocks = parse(&html);
    let counts = Counts::of(&blocks);
    assert_eq!(counts.empty, 0, "{name} has a heading with nothing in it");
    assert!(
        counts.headings.len() > 1000,
        "{name} has {} headings",
        counts.headings.len()
    );
    assert!(
        counts.headings.iter().any(|level| *level > 2),
        "{name} came out with a flat outline, so the sections were not counted"
    );
    assert!(
        counts.paragraphs > 1000,
        "{name} has {} paragraphs",
        counts.paragraphs
    );
    assert!(counts.lists > 1000, "{name} has {} lists", counts.lists);
    assert!(counts.tables > 50, "{name} has {} tables", counts.tables);
    assert!(
        counts.code > 1000,
        "{name} has {} blocks of preformatted text",
        counts.code
    );
}

#[test]
fn nothing_of_the_document_is_left_as_markup() {
    let Some(path) = documents().first().cloned() else {
        return;
    };
    let Ok(html) = std::fs::read_to_string(&path) else {
        return;
    };
    let blocks = parse(&html);
    let mut text = String::new();
    words(&blocks, &mut text);
    // Not `</`: the specification writes markup about markup, and the
    // string `"</"` is a step of one of its algorithms.
    for tag in ["<p>", "<emu-", "</p>", "&amp;", "&lt;"] {
        assert!(
            !text.contains(tag),
            "{} still carries `{tag}`",
            path.display()
        );
    }
}

fn words(blocks: &[Block], out: &mut String) {
    for block in blocks {
        match block {
            Block::Heading { content, .. } | Block::Paragraph(content) => {
                out.push_str(&plain(content));
            }
            Block::Code { lines, .. } => {
                for line in lines {
                    out.push_str(line);
                }
            }
            Block::Quote(inner) => words(inner, out),
            Block::List { items, .. } => {
                for item in items {
                    words(item, out);
                }
            }
            Block::Table { header, rows, .. } => {
                for cell in header.iter().chain(rows.iter().flatten()) {
                    out.push_str(&plain(cell));
                }
            }
            Block::Rule => {}
        }
    }
}
