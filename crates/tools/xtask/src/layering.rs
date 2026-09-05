// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The layering check: dependency edges against the policy, the
//! `forbid(unsafe_code)` attribute in logic crates, and the absence of
//! assembly files.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::fs;
use crate::policy::{ASSEMBLY_EXTENSIONS, CRATES, DEV_DEPENDENCIES, Kind, find};
use crate::process::Cmd;
use crate::unsafe_budget;

/// A dependency edge between two packages.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Edge {
    /// The dependent package.
    pub(crate) from: String,
    /// The dependency.
    pub(crate) to: String,
}

/// Runs every layering check and returns the violations.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, Error> {
    let mut violations = Vec::new();
    violations.extend(check_crate_list(root)?);
    let normal = cargo_tree(root, "normal,build", None)?;
    violations.extend(check_edges(&parse_tree(&normal), false));
    let dev = cargo_tree(root, "dev", Some(1))?;
    violations.extend(check_edges(&parse_tree(&dev), true));
    violations.extend(check_crate_roots(root)?);
    violations.extend(check_assembly(root)?);
    Ok(violations)
}

fn cargo_tree(root: &Path, edges: &str, depth: Option<u32>) -> Result<String, Error> {
    let mut cmd = Cmd::cargo()
        .cwd(root)
        .args([
            "tree",
            "--workspace",
            "--all-features",
            "--prefix",
            "depth",
            "--format",
            "{p}",
        ])
        .arg("--edges")
        .arg(edges);
    if let Some(depth) = depth {
        cmd = cmd.arg("--depth").arg(depth.to_string());
    }
    cmd.capture()
}

/// Parses the depth-prefixed output of `cargo tree` into edges. Group
/// markers such as `[dev-dependencies]` attach their children to the
/// enclosing package.
pub(crate) fn parse_tree(output: &str) -> BTreeSet<Edge> {
    let mut edges = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();
    for line in output.lines() {
        let digits = line
            .len()
            .saturating_sub(line.trim_start_matches(|c: char| c.is_ascii_digit()).len());
        let Some((depth_text, rest)) = line.split_at_checked(digits) else {
            continue;
        };
        let Ok(depth) = depth_text.parse::<usize>() else {
            continue;
        };
        let name = rest.split_whitespace().next().unwrap_or("").to_owned();
        if name.is_empty() {
            continue;
        }
        stack.truncate(depth);
        if name.starts_with('[') {
            let parent = depth
                .checked_sub(1)
                .and_then(|i| stack.get(i).cloned())
                .unwrap_or_default();
            stack.push(parent);
            continue;
        }
        if let Some(parent) = depth.checked_sub(1).and_then(|i| stack.get(i))
            && parent != &name
        {
            edges.insert(Edge {
                from: parent.clone(),
                to: name.clone(),
            });
        }
        stack.push(name);
    }
    edges
}

/// Checks edges against the policy. Dev edges may additionally use the
/// crates listed in `DEV_DEPENDENCIES`.
pub(crate) fn check_edges(edges: &BTreeSet<Edge>, dev: bool) -> Vec<String> {
    let mut violations = Vec::new();
    for edge in edges {
        let Some(from) = find(&edge.from) else {
            violations.push(format!("`{}` is not in the policy table", edge.from));
            continue;
        };
        if find(&edge.to).is_none() {
            violations.push(format!(
                "`{}` depends on `{}`, which is outside the workspace",
                edge.from, edge.to
            ));
            continue;
        }
        let allowed = from.deps.contains(&edge.to.as_str())
            || (dev && DEV_DEPENDENCIES.contains(&edge.to.as_str()));
        if !allowed {
            let kind = if dev { "dev-dependency" } else { "dependency" };
            violations.push(format!(
                "`{}` has the undeclared {kind} `{}`",
                edge.from, edge.to
            ));
        }
    }
    violations
}

/// The workspace members must equal the policy table, and every path must
/// hold a crate with the expected name.
fn check_crate_list(root: &Path) -> Result<Vec<String>, Error> {
    let mut violations = Vec::new();
    let manifest = fs::read(&root.join("Cargo.toml"))?;
    let members = workspace_members(&manifest);
    for member in &members {
        if !CRATES.iter().any(|c| c.path == member) {
            violations.push(format!(
                "workspace member `{member}` is not in the policy table"
            ));
        }
    }
    for krate in CRATES {
        if !members.iter().any(|m| m == krate.path) {
            violations.push(format!(
                "policy crate `{}` is not a workspace member",
                krate.name
            ));
        }
        let crate_manifest = root.join(krate.path).join("Cargo.toml");
        match fs::read(&crate_manifest) {
            Ok(text) if package_name(&text).as_deref() == Some(krate.name) => {}
            Ok(text) => violations.push(format!(
                "`{}` declares package `{}`, policy expects `{}`",
                krate.path,
                package_name(&text).unwrap_or_default(),
                krate.name
            )),
            Err(_) => violations.push(format!("`{}` has no Cargo.toml", krate.path)),
        }
    }
    Ok(violations)
}

/// The `members` of the workspace manifest.
pub(crate) fn workspace_members(manifest: &str) -> Vec<String> {
    let Some(start) = manifest.find("members") else {
        return Vec::new();
    };
    let Some(open) = manifest
        .get(start..)
        .and_then(|s| s.find('['))
        .map(|i| i.saturating_add(start))
    else {
        return Vec::new();
    };
    let Some(close) = manifest
        .get(open..)
        .and_then(|s| s.find(']'))
        .map(|i| i.saturating_add(open))
    else {
        return Vec::new();
    };
    manifest
        .get(open.saturating_add(1)..close)
        .unwrap_or("")
        .split(',')
        .map(|item| item.trim().trim_matches('"').to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

/// The `name` of the `[package]` section.
pub(crate) fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package && let Some(rest) = line.strip_prefix("name") {
            let value = rest
                .trim_start()
                .strip_prefix('=')?
                .trim()
                .trim_matches('"');
            return Some(value.to_owned());
        }
    }
    None
}

/// Logic and host crates carry `#![forbid(unsafe_code)]`; adapter crates
/// carry `#![allow(unsafe_code)]`.
fn check_crate_roots(root: &Path) -> Result<Vec<String>, Error> {
    let mut violations = Vec::new();
    for krate in CRATES {
        let src = root.join(krate.path).join("src");
        let wanted = match krate.kind {
            Kind::Logic | Kind::Host => "#![forbid(unsafe_code)]",
            Kind::Adapter { .. } => "#![allow(unsafe_code)]",
        };
        for crate_root in crate_roots(&src)? {
            let text = fs::read(&crate_root)?;
            if !text.lines().any(|line| line.trim() == wanted) {
                let name = fs::file_name(&crate_root);
                violations.push(format!(
                    "`{}` lacks `{wanted}` in its crate root {name}",
                    krate.name
                ));
            }
        }
    }
    Ok(violations)
}

/// Every crate root of a package: its library or its binary, and every
/// binary of a package that is nothing but binaries, as the user test
/// programs are.
fn crate_roots(src: &Path) -> Result<Vec<PathBuf>, Error> {
    if src.join("lib.rs").is_file() {
        return Ok(vec![src.join("lib.rs")]);
    }
    if src.join("main.rs").is_file() {
        return Ok(vec![src.join("main.rs")]);
    }
    let bins = src.join("bin");
    let mut roots: Vec<PathBuf> = fs::walk_files(&bins)?
        .into_iter()
        .filter(|path| fs::extension(path) == "rs")
        .collect();
    roots.sort();
    if roots.is_empty() {
        return Err(Error::Parse(format!(
            "{} holds no crate root",
            src.display()
        )));
    }
    Ok(roots)
}

/// No assembly files and no `global_asm!` anywhere.
fn check_assembly(root: &Path) -> Result<Vec<String>, Error> {
    let mut violations = Vec::new();
    for path in fs::walk_files(root)? {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if ASSEMBLY_EXTENSIONS.contains(&fs::extension(&path)) {
            violations.push(format!("assembly file `{relative}` is not allowed"));
        }
        if fs::extension(&path) == "rs"
            && unsafe_budget::count(&fs::read(&path)?).global_asm_macros > 0
        {
            violations.push(format!("`{relative}` uses `global_asm!`"));
        }
    }
    Ok(violations)
}
