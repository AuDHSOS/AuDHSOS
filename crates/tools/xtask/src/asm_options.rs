// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The `nomem` rule for assembly that changes the interrupt state (R11 of
//! document 4).
//!
//! `nomem` tells the compiler that the block reads and writes no memory
//! outside itself, which lets it cache a global in a register across the
//! block and move a load or a store to the other side of it. `cli` and
//! `sti` are what an interrupt guard is built of, so a store the guard was
//! meant to cover may end up outside it, where a handler sees it. Such a
//! block must be a full compiler barrier, which is what an `asm!` without
//! `nomem` is.
//!
//! Only adapter crates are read: a logic or host crate carries no
//! assembly at all, which `check-layering` and the unsafe budget enforce,
//! and the sources of this check are host code.

use std::path::Path;

use crate::error::Error;
use crate::fs;
use crate::policy::{CRATES, Kind};

/// The instructions that may not stand in a block declaring `nomem`.
const BARRIER_INSTRUCTIONS: [&str; 2] = ["cli", "sti"];

/// What opens an assembly block; `naked_asm!(` ends in the same five
/// characters.
const OPENING: &str = "asm!(";

/// Checks the Rust files of every adapter crate below `root` and returns
/// the violations.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, Error> {
    let mut violations = Vec::new();
    for krate in CRATES {
        if !matches!(krate.kind, Kind::Adapter { .. }) {
            continue;
        }
        let directory = root.join(krate.path);
        if !directory.is_dir() {
            continue;
        }
        for path in fs::walk_files(&directory)? {
            if fs::extension(&path) != "rs" {
                continue;
            }
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let content = fs::read(&path)?;
            for problem in problems(&content) {
                violations.push(format!("{}: {problem}", relative.display()));
            }
        }
    }
    Ok(violations)
}

/// The blocks of `source` that name a barrier instruction and declare
/// `nomem`, as one message each.
pub(crate) fn problems(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes: Vec<char> = source.chars().collect();
    let mut start = 0;
    while let Some(offset) = find_from(&bytes, start, OPENING) {
        let body_start = offset.saturating_add(OPENING.chars().count());
        let body_end = closing_paren(&bytes, body_start);
        let body: String = bytes
            .get(body_start..body_end)
            .unwrap_or(&[])
            .iter()
            .collect();
        if body.contains("nomem") {
            for instruction in BARRIER_INSTRUCTIONS {
                if names_instruction(&body, instruction) {
                    found.push(format!(
                        "line {}: the `{instruction}` block declares `nomem`, \
                         which lets the compiler move memory accesses across it",
                        line_of(&bytes, offset)
                    ));
                }
            }
        }
        start = body_start;
    }
    found
}

/// The index of `needle` in `text` at or after `from`, in characters.
fn find_from(text: &[char], from: usize, needle: &str) -> Option<usize> {
    let wanted: Vec<char> = needle.chars().collect();
    let last = text.len().checked_sub(wanted.len())?;
    (from..=last)
        .find(|index| text.get(*index..index.saturating_add(wanted.len())) == Some(&wanted))
}

/// The index of the parenthesis that closes the block whose body starts at
/// `from`, or the end of the text for an unterminated block. Parentheses
/// inside a string literal do not count.
fn closing_paren(text: &[char], from: usize) -> usize {
    let mut depth = 1_usize;
    let mut index = from;
    while index < text.len() {
        match text.get(index) {
            Some('"') => index = skip_string(text, index),
            Some('(') => {
                depth = depth.saturating_add(1);
                index = index.saturating_add(1);
            }
            Some(')') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index;
                }
                index = index.saturating_add(1);
            }
            _ => index = index.saturating_add(1),
        }
    }
    text.len()
}

/// The index one past the string literal that starts at `from`.
fn skip_string(text: &[char], from: usize) -> usize {
    let mut index = from.saturating_add(1);
    while index < text.len() {
        match text.get(index) {
            Some('\\') => index = index.saturating_add(2),
            Some('"') => return index.saturating_add(1),
            _ => index = index.saturating_add(1),
        }
    }
    text.len()
}

/// The one-based line the character at `offset` stands on.
fn line_of(text: &[char], offset: usize) -> usize {
    text.get(..offset)
        .unwrap_or(&[])
        .iter()
        .filter(|c| **c == '\n')
        .count()
        .saturating_add(1)
}

/// `true` if `body` carries `instruction` as a whole word, so that
/// `nostack` does not match `sti`.
fn names_instruction(body: &str, instruction: &str) -> bool {
    let mut rest = body;
    while let Some(offset) = rest.find(instruction) {
        let before = rest.get(..offset).unwrap_or("");
        let after = rest
            .get(offset.saturating_add(instruction.len())..)
            .unwrap_or("");
        let left_free = before.chars().next_back().is_none_or(is_boundary);
        let right_free = after.chars().next().is_none_or(is_boundary);
        if left_free && right_free {
            return true;
        }
        rest = after;
    }
    false
}

/// `true` if `c` ends a word of an assembly template.
fn is_boundary(c: char) -> bool {
    !(c.is_alphanumeric() || c == '_')
}
