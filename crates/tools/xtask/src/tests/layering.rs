// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::layering`.

use crate::layering::{Edge, check_edges, package_name, parse_tree, workspace_members};
use std::collections::BTreeSet;

const TREE: &str = "\
0kernel-types v0.1.0 (/w/crates/kernel/types)
1audhsos-abi v0.1.0 (/w/crates/abi)
1test-support v0.1.0 (/w/crates/support/testing)
1[dev-dependencies]
2test-support v0.1.0 (/w/crates/support/testing) (*)
0audhsos-abi v0.1.0 (/w/crates/abi)
";

fn edge(from: &str, to: &str) -> Edge {
    Edge {
        from: from.to_owned(),
        to: to.to_owned(),
    }
}

#[test]
fn nested_prefixes_and_group_markers_produce_edges() {
    let edges = parse_tree(TREE);
    assert!(edges.contains(&edge("kernel-types", "audhsos-abi")));
    assert!(edges.contains(&edge("kernel-types", "test-support")));
    assert_eq!(edges.len(), 2);
}

#[test]
fn repeated_crates_and_garbage_lines_are_handled() {
    let edges = parse_tree("0a v1\n1b v1\n2c v1 (*)\n0b v1\n1c v1\n\nnot-a-tree-line\n1\n");
    assert_eq!(edges, BTreeSet::from([edge("a", "b"), edge("b", "c")]));
}

#[test]
fn edges_are_checked_against_the_policy() {
    let ok = BTreeSet::from([edge("kernel-types", "audhsos-abi")]);
    assert!(check_edges(&ok, false).is_empty());
    let upward = BTreeSet::from([edge("audhsos-abi", "kernel-types")]);
    assert_eq!(check_edges(&upward, false).len(), 1);
    let external = BTreeSet::from([edge("audhsos-abi", "serde")]);
    assert!(check_edges(&external, false)[0].contains("outside the workspace"));
    let unknown = BTreeSet::from([edge("mystery", "audhsos-abi")]);
    assert!(check_edges(&unknown, false)[0].contains("not in the policy"));
    let dev = BTreeSet::from([edge("kernel-hal-api", "test-support")]);
    assert!(check_edges(&dev, true).is_empty());
    assert_eq!(check_edges(&dev, false).len(), 1);
}

#[test]
fn workspace_members_and_package_names_are_parsed() {
    let manifest = "[workspace]\nmembers = [\n    \"crates/abi\",\n    \"crates/sync\",\n]\n";
    assert_eq!(
        workspace_members(manifest),
        vec!["crates/abi", "crates/sync"]
    );
    assert!(workspace_members("[workspace]\n").is_empty());
    assert!(workspace_members("members = [").is_empty());
    let package =
        "[package]\nname = \"audhsos-abi\"\nversion = \"1\"\n[dependencies]\nname = \"x\"\n";
    assert_eq!(package_name(package).as_deref(), Some("audhsos-abi"));
    assert_eq!(package_name("[dependencies]\nname = \"x\"\n"), None);
    assert_eq!(package_name("[package]\nname \"x\"\n"), None);
}
