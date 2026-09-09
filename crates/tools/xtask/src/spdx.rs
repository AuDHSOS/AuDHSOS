// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The SPDX header check.

use std::path::Path;

use crate::error::Error;
use crate::fs;
use crate::policy::{
    HEADER_EXEMPT_FILES, HEADER_FILE_TYPES, PORTED_FILES, PORTED_HEADER, SPDX_HEADER,
};

/// Checks every file with a known extension below `root` and returns the
/// violations.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, Error> {
    let mut violations = Vec::new();
    for path in fs::walk_files(root)? {
        let relative = path.strip_prefix(root).unwrap_or(&path);
        if upstream_reference(relative) {
            continue;
        }
        let Some(prefix) = comment_prefix(&path) else {
            continue;
        };
        let content = fs::read(&path)?;
        if let Some(problem) = header_problem(&content, prefix, expected_header(relative)) {
            violations.push(format!("{}: {problem}", relative.display()));
        }
    }
    Ok(violations)
}

/// Verbatim third-party conformance inputs retain upstream copyright/licensing.
/// This exemption is root-relative and intentionally not a filename wildcard.
pub(crate) fn upstream_reference(relative: &Path) -> bool {
    relative.starts_with("docs/test-ext/test262")
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

/// The header `relative` must carry: the one that names both licences for
/// a file that is in part a port, and the project's own for every other.
pub(crate) fn expected_header(relative: &Path) -> &'static [&'static str] {
    let path = relative.to_string_lossy().replace('\\', "/");
    if PORTED_FILES.contains(&path.as_str()) {
        &PORTED_HEADER
    } else {
        &SPDX_HEADER
    }
}

/// What is wrong with the header of `content`, if anything.
pub(crate) fn header_problem(
    content: &str,
    prefix: &str,
    expected_lines: &[&str],
) -> Option<String> {
    let mut lines = content.lines();
    for (index, expected) in expected_lines.iter().enumerate() {
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
