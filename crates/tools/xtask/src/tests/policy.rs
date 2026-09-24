// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::policy`.

use crate::policy::{CRATES, DEV_DEPENDENCIES, Kind, MIRI_TARGETS, find};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[test]
fn names_and_paths_are_unique() {
    let names: HashSet<&str> = CRATES.iter().map(|c| c.name).collect();
    let paths: HashSet<&str> = CRATES.iter().map(|c| c.path).collect();
    assert_eq!(names.len(), CRATES.len());
    assert_eq!(paths.len(), CRATES.len());
}

#[test]
fn every_dependency_names_a_known_crate() {
    for krate in CRATES {
        for dep in krate.deps {
            assert!(
                find(dep).is_some(),
                "{} depends on unknown {dep}",
                krate.name
            );
        }
    }
    for dep in DEV_DEPENDENCIES {
        assert!(find(dep).is_some());
    }
    for target in MIRI_TARGETS {
        assert!(find(target.name).is_some());
    }
}

#[test]
fn dependencies_never_point_upward_in_layer_order() {
    for (index, krate) in CRATES.iter().enumerate() {
        for dep in krate.deps {
            let position = CRATES.iter().position(|c| c.name == *dep).unwrap();
            assert!(position != index, "{} depends on itself", krate.name);
        }
    }
}

#[test]
fn only_adapters_have_budgets() {
    for krate in CRATES {
        if let Kind::Adapter { unsafe_budget, .. } = krate.kind {
            assert!(unsafe_budget > 0, "{} has an empty budget", krate.name);
        }
    }
    assert!(find("missing").is_none());
}

#[test]
fn text_demo_is_the_documented_workspace_lint_exception() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let guide = fs::read_to_string(root.join("docs/05-code-organization.md")).unwrap();
    let layout = guide.split("## 5.1 Repository layout").nth(1).unwrap();
    let catalog = guide
        .split("## 5.2 Crate catalog")
        .nth(1)
        .unwrap()
        .split("## 5.3")
        .next()
        .unwrap();
    let configuration = guide
        .split("## 5.4 Workspace configuration")
        .nth(1)
        .unwrap()
        .split("## 5.5")
        .next()
        .unwrap();

    assert!(layout.contains("├── text-demo/"));
    assert!(
        catalog
            .lines()
            .any(|line| line.starts_with("| `text-demo` |"))
    );
    assert!(configuration.contains("except `text-demo` declares `[lints] workspace = true`"));
    let safety = fs::read_to_string(root.join("docs/04-safety-policy.md")).unwrap();
    assert!(safety.contains("R7's lint enforcement excludes the throwaway `text-demo` crate"));

    for krate in CRATES {
        let manifest = fs::read_to_string(root.join(krate.path).join("Cargo.toml")).unwrap();
        if krate.name == "text-demo" {
            assert!(!manifest.lines().any(|line| line.trim() == "[lints]"));
        } else {
            assert!(
                manifest.contains("[lints]\nworkspace = true"),
                "{} must inherit workspace lints",
                krate.name
            );
        }
    }
}

#[test]
fn user_test_programs_do_not_suppress_dead_code() {
    for (path, source) in user_test_programs() {
        assert!(
            !source.contains("dead_code"),
            "{} suppresses dead code",
            path.display()
        );
    }
}

#[test]
fn user_test_program_safety_sections_document_unsafe_functions() {
    for (path, source) in user_test_programs() {
        let mut lines = source.lines();
        while let Some(line) = lines.next() {
            if line.trim() != "/// # Safety" {
                continue;
            }
            let item = lines
                .by_ref()
                .map(str::trim)
                .find(|line| {
                    !line.is_empty() && !line.starts_with("///") && !line.starts_with("#[")
                })
                .unwrap_or("");
            assert!(
                item.starts_with("unsafe fn ")
                    || item.starts_with("pub unsafe fn ")
                    || item.starts_with("pub unsafe extern "),
                "{} has a Safety section on `{item}`",
                path.display()
            );
        }
    }
}

fn user_test_programs() -> Vec<(std::path::PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let directory = root.join("crates/user/test-programs/src/bin");
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let source = fs::read_to_string(&path).unwrap();
            (path, source)
        })
        .collect()
}

#[test]
fn every_feature_set_names_a_cross_crate_and_its_declared_features() {
    use crate::policy::{FEATURE_SETS, Target};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for &(name, features) in FEATURE_SETS {
        let krate = find(name).unwrap_or_else(|| panic!("{name} is not a crate"));
        assert_ne!(krate.target, Target::Host, "{name} is a host crate");
        let manifest = fs::read_to_string(root.join(krate.path).join("Cargo.toml")).unwrap();
        for feature in features.split(',').filter(|feature| !feature.is_empty()) {
            assert!(
                manifest.contains(&format!("\n{feature} = ")),
                "{name} declares no feature {feature}"
            );
        }
    }
}

#[test]
fn hal_functions_that_rely_on_a_caller_promise_are_unsafe() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let source = |path: &str| fs::read_to_string(root.join(path)).unwrap();
    // Issue #87.
    let testing = source("crates/kernel/hal-x86_64/src/testing.rs");
    assert!(testing.contains("pub unsafe fn read_byte(address: u64) -> u8"));
    assert!(testing.contains("pub unsafe fn write_byte(address: u64, value: u8)"));
    // Issue #108.
    let ports = source("crates/kernel/hal-x86_64/src/ports.rs");
    assert!(ports.contains("#[derive(Clone, Copy, Debug)]\npub struct Ports(());"));
    assert!(ports.contains("pub const unsafe fn new() -> Self"));
    assert!(ports.contains("pub const unsafe fn new(apics: &'a mut crate::apic::Apics) -> Self"));
}
