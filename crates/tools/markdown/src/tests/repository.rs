// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The documents of this repository, which are the input the parser has to
//! be right about. The tests state what has to hold for every one of them,
//! and say which document broke it.

use std::path::{Path, PathBuf};

use crate::block::Block;
use crate::inline::plain;
use crate::parse::document;

/// The workspace root, from the manifest directory of this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Every Markdown file of the repository, outside what a build writes.
fn documents() -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(&root(), &mut found);
    found.sort();
    found
}

fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if !["target", ".git", "research", ".claude", "fuzz"]
                .iter()
                .any(|skip| name == *skip)
            {
                collect(&path, found);
            }
        } else if path.extension().is_some_and(|ext| ext == "md") {
            found.push(path);
        }
    }
}

#[test]
fn the_repository_has_documents_to_parse() {
    assert!(
        documents().len() > 20,
        "the crate READMEs alone should be more than twenty"
    );
}

#[test]
fn every_document_parses_into_blocks() {
    for path in documents() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        assert!(
            !document(&text).is_empty(),
            "{} parsed into nothing",
            path.display()
        );
    }
}

#[test]
fn no_document_loses_a_fence() {
    for path in documents() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let fences = text
            .lines()
            .filter(|line| line.trim_start().starts_with("```"))
            .count();
        if fences == 0 || fences.wrapping_rem(2) != 0 {
            continue;
        }
        let blocks = count_code(&document(&text));
        assert!(
            blocks >= fences.wrapping_div(2),
            "{} has {fences} fence lines but only {blocks} code blocks",
            path.display()
        );
    }
}

fn count_code(blocks: &[Block]) -> usize {
    let mut total = 0usize;
    for block in blocks {
        match block {
            Block::Code { .. } => total = total.saturating_add(1),
            Block::Quote(inner) => total = total.saturating_add(count_code(inner)),
            Block::List { items, .. } => {
                for item in items {
                    total = total.saturating_add(count_code(item));
                }
            }
            _ => {}
        }
    }
    total
}

#[test]
fn every_document_that_starts_with_a_heading_keeps_it() {
    for path in documents() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(first) = text.lines().find(|line| !line.trim().is_empty()) else {
            continue;
        };
        let Some(title) = first.strip_prefix("# ") else {
            continue;
        };
        let blocks = document(&text);
        assert_eq!(
            blocks.first().and_then(Block::heading_text).as_deref(),
            Some(title.trim()),
            "{} lost its title",
            path.display()
        );
    }
}

#[test]
fn a_heading_never_comes_out_empty() {
    for path in documents() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for block in document(&text) {
            if let Block::Heading { content, .. } = &block {
                assert!(
                    !plain(content).trim().is_empty(),
                    "{} has an empty heading",
                    path.display()
                );
            }
        }
    }
}
