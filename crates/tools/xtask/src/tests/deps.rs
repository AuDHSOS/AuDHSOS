// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::deps`.

use crate::deps::{lock_violations, manifest_violations};

#[test]
fn lock_packages_with_a_source_are_reported() {
    let lock = "version = 4\n\n[[package]]\nname = \"audhsos-abi\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n";
    let violations = lock_violations(lock);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].contains("serde"));
    assert!(lock_violations("version = 4\n").is_empty());
}

#[test]
fn manifest_dependencies_must_be_workspace_or_path() {
    let good = "[package]\nname = \"a\"\n[dependencies]\nb.workspace = true\nc = { workspace = true, optional = true }\nd = { path = \"../d\" }\n[dev-dependencies]\ne = { workspace = true }\n[lints]\nworkspace = true\n";
    assert!(manifest_violations(good).is_empty());
    let bad = "[dependencies]\nserde = \"1\"\ng = { git = \"https://x\" }\nh = { path = \"../h\", version = \"1\" }\n[target.'cfg(unix)'.dev-dependencies]\nlibc = \"0.2\"\n[build-dependencies]\ncc = { version = \"1\" }\n";
    let violations = manifest_violations(bad);
    assert_eq!(violations.len(), 5);
    assert!(
        violations
            .iter()
            .all(|v| v.contains("not a workspace member"))
    );
}

#[test]
fn comments_blank_lines_and_other_sections_are_ignored() {
    let manifest = "[dependencies]\n# comment\n\n[features]\nstd = []\ndefault = [\"std\"]\n";
    assert!(manifest_violations(manifest).is_empty());
}
