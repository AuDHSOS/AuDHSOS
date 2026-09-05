// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Links between documents, rewritten to point at what was written.
//!
//! The documents of this repository refer to each other by their paths in
//! the source tree: the handbook links to `12-parallel-work.md`, a crate
//! README links to `../../../docs/04-safety-policy.md`. Converted as they
//! stand, every one of those links would point at a Markdown file beside a
//! set of PDFs. So a link whose target is a document that was converted is
//! rewritten to the file the conversion wrote, relative to the document
//! doing the linking. A link to anything else — an address on the web, a
//! source file, a directory — is left exactly as it was.

use std::collections::BTreeMap;

use crate::sources::Source;

/// Where every converted document came from and where it went.
#[derive(Clone, Debug, Default)]
pub(crate) struct Links {
    /// Source path relative to the root, to target path relative to the
    /// output directory.
    map: BTreeMap<String, String>,
}

impl Links {
    /// The table for a set of sources.
    pub(crate) fn new(sources: &[Source]) -> Self {
        let map = sources
            .iter()
            .map(|source| {
                (
                    source.origin.clone(),
                    source.target.to_string_lossy().replace('\\', "/"),
                )
            })
            .collect();
        Self { map }
    }

    /// The address a link gets in the written document.
    #[must_use]
    pub(crate) fn resolve(&self, from: &Source, href: &str) -> String {
        if href.contains("://") || href.starts_with("mailto:") || href.starts_with('#') {
            return href.to_owned();
        }
        let (path, _anchor) = href.split_once('#').unwrap_or((href, ""));
        if path.is_empty() {
            return href.to_owned();
        }
        let origin = from.origin.clone();
        let Some(source) = normalise(parent(&origin), path) else {
            return href.to_owned();
        };
        let Some(target) = self.map.get(&source) else {
            return href.to_owned();
        };
        let own = from.target.to_string_lossy().replace('\\', "/");
        relative(parent(&own), target)
    }
}

/// The directory part of a path, without its trailing separator.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Joins a relative reference onto a directory and resolves `.` and `..`.
/// A reference that climbs above the root has no answer here.
fn normalise(dir: &str, path: &str) -> Option<String> {
    let mut parts: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        dir.split('/').filter(|part| !part.is_empty()).collect()
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// The path from a directory to a file, both relative to the same root.
fn relative(from: &str, to: &str) -> String {
    let from: Vec<&str> = from.split('/').filter(|part| !part.is_empty()).collect();
    let to: Vec<&str> = to.split('/').filter(|part| !part.is_empty()).collect();
    let shared = from
        .iter()
        .zip(to.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<String> = from
        .get(shared..)
        .unwrap_or_default()
        .iter()
        .map(|_| "..".to_owned())
        .collect();
    parts.extend(
        to.get(shared..)
            .unwrap_or_default()
            .iter()
            .map(|part| (*part).to_owned()),
    );
    parts.join("/")
}
