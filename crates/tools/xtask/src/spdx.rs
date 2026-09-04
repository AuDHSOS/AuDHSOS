// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The SPDX header check.

use std::path::Path;

use crate::error::Error;
use crate::fs;
use crate::policy::{HEADER_EXEMPT_FILES, HEADER_FILE_TYPES, SPDX_HEADER};

/// Checks every file with a known extension below `root` and returns the
/// violations.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, Error> {
    let mut violations = Vec::new();
    for path in fs::walk_files(root)? {
        let Some(prefix) = comment_prefix(&path) else {
            continue;
        };
        let content = fs::read(&path)?;
        if let Some(problem) = header_problem(&content, prefix) {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            violations.push(format!("{}: {problem}", relative.display()));
        }
    }
    Ok(violations)
}

/// The comment prefix for a file that needs the header, or `None` if the
/// file is exempt or of an unknown type.
pub(crate) fn comment_prefix(path: &Path) -> Option<&'static str> {
    if HEADER_EXEMPT_FILES.contains(&fs::file_name(path)) {
        return None;
    }
    let extension = fs::extension(path);
    HEADER_FILE_TYPES
        .iter()
        .find(|(ext, _)| *ext == extension)
        .map(|(_, prefix)| *prefix)
}

/// What is wrong with the header of `content`, if anything.
pub(crate) fn header_problem(content: &str, prefix: &str) -> Option<String> {
    let mut lines = content.lines();
    for (index, expected) in SPDX_HEADER.iter().enumerate() {
        let wanted = format!("{prefix} {expected}");
        match lines.next() {
            Some(line) if line == wanted => {}
            Some(line) => {
                return Some(format!(
                    "line {} is `{line}`, expected `{wanted}`",
                    index.saturating_add(1)
                ));
            }
            None => return Some(format!("missing header line {}", index.saturating_add(1))),
        }
    }
    None
}
