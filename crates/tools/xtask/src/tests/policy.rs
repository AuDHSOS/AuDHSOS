// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::policy`.

use crate::policy::{CRATES, DEV_DEPENDENCIES, Kind, MIRI_CRATES, find};
use std::collections::HashSet;

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
    for dep in DEV_DEPENDENCIES.iter().chain(MIRI_CRATES) {
        assert!(find(dep).is_some());
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
