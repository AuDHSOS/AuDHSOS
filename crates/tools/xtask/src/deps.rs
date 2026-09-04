// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The external-code check: `Cargo.lock` and every manifest reference
//! workspace members only.

use std::path::Path;

use crate::error::Error;
use crate::fs;

/// Runs the check and returns the violations.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, Error> {
    let mut violations = lock_violations(&fs::read(&root.join("Cargo.lock"))?);
    for path in fs::walk_files(root)? {
        if fs::file_name(&path) == "Cargo.toml" {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            for problem in manifest_violations(&fs::read(&path)?) {
                violations.push(format!("{relative}: {problem}"));
            }
        }
    }
    Ok(violations)
}

/// Packages in a lock file that have a `source`, which only packages from
/// outside the workspace have.
pub(crate) fn lock_violations(lock: &str) -> Vec<String> {
    let mut violations = Vec::new();
    for block in lock.split("[[package]]").skip(1) {
        let name = value_of(block, "name").unwrap_or_default();
        if let Some(source) = value_of(block, "source") {
            violations.push(format!(
                "Cargo.lock: package `{name}` comes from `{source}`"
            ));
        }
    }
    violations
}

fn value_of(block: &str, key: &str) -> Option<String> {
    block.lines().map(str::trim).find_map(|line| {
        let rest = line
            .strip_prefix(key)?
            .trim_start()
            .strip_prefix('=')?
            .trim();
        Some(rest.trim_matches('"').to_owned())
    })
}

/// Section headers whose entries are dependencies.
fn is_dependency_section(header: &str) -> bool {
    let name = header.trim_matches(|c| c == '[' || c == ']');
    let last = name.rsplit('.').next().unwrap_or(name);
    matches!(
        last,
        "dependencies" | "dev-dependencies" | "build-dependencies"
    )
}

/// Dependency entries of a manifest that do not point at the workspace.
pub(crate) fn manifest_violations(manifest: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let mut in_dependencies = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_dependencies = is_dependency_section(line);
            continue;
        }
        if !in_dependencies || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let ok = key.ends_with(".workspace")
            || value.contains("workspace = true")
            || (value.starts_with('{') && value.contains("path =") && !mentions_external(value));
        if !ok {
            violations.push(format!(
                "dependency `{key}` is not a workspace member: `{value}`"
            ));
        }
    }
    violations
}

fn mentions_external(value: &str) -> bool {
    [
        "version =",
        "git =",
        "registry =",
        "branch =",
        "rev =",
        "tag =",
    ]
    .iter()
    .any(|k| value.contains(k))
}
