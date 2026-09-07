// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What gets converted, and where it lands.
//!
//! The tests build a small repository in a directory of their own rather
//! than reading the real one: the rules are about shapes of paths, and a
//! test that asserted the number of crate READMEs in this repository would
//! fail every time somebody wrote one.

use std::path::{Path, PathBuf};

use crate::sources::{Kind, collect};

/// A repository built for one test, removed when the test ends.
struct Scratch {
    /// Where it lives.
    path: PathBuf,
}

impl Scratch {
    /// An empty directory named after the test using it.
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("audhsos-docpdf-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::create_dir_all(&path);
        Self { path }
    }

    /// Writes a file, creating the directories above it.
    fn write(&self, relative: &str, text: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, text);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The target paths a repository produces, as text, in order.
fn targets(root: &Path) -> Vec<String> {
    collect(root)
        .unwrap_or_default()
        .iter()
        .map(|source| source.target.to_string_lossy().into_owned())
        .collect()
}

fn repository(name: &str) -> Scratch {
    let scratch = Scratch::new(name);
    scratch.write("README.md", "# The Project\n\nWhat it is.\n");
    scratch.write("CHANGELOG.md", "# Changelog\n");
    scratch.write("docs/README.md", "# Contents\n");
    scratch.write("docs/01-vision.md", "# 1. Vision\n");
    scratch.write("docs/rfc/README.md", "# The documents\n");
    scratch.write("docs/rfc/rfc791.txt", "Internet Protocol\n");
    scratch.write("crates/net/ip/Cargo.toml", "[package]\nname = \"net-ip\"\n");
    scratch.write("crates/net/ip/README.md", "# net-ip\n\nIPv4.\n");
    scratch.write("tools/probe/README.md", "# probe\n");
    scratch.write(
        "docs/ecma/spec.html",
        "<html><head><title>A Standard</title></head><body><h1>Clause</h1></body></html>",
    );
    scratch
}

#[test]
fn a_document_is_filed_by_what_it_is() {
    let scratch = repository("filed");
    let found = targets(&scratch.path);
    for wanted in [
        "project/readme.pdf",
        "project/changelog.pdf",
        "handbook/00-contents.pdf",
        "handbook/01-vision.pdf",
        "crates/net-ip.pdf",
        "tools/probe.pdf",
        "rfc/00-index.pdf",
        "rfc/rfc791.pdf",
        "spec/spec.pdf",
    ] {
        assert!(
            found.iter().any(|target| target == wanted),
            "{wanted} is missing from {found:?}"
        );
    }
}

#[test]
fn a_crate_is_named_after_its_package_and_not_after_its_directory() {
    let scratch = repository("named");
    let found = targets(&scratch.path);
    assert!(found.iter().any(|target| target == "crates/net-ip.pdf"));
    assert!(
        !found
            .iter()
            .any(|target| target.contains("readme") && target.starts_with("crates/"))
    );
}

#[test]
fn a_crate_with_no_manifest_falls_back_to_its_directory() {
    let scratch = repository("unnamed");
    scratch.write("crates/loose/README.md", "# loose\n");
    assert!(
        targets(&scratch.path)
            .iter()
            .any(|target| target == "crates/loose.pdf")
    );
}

#[test]
fn a_document_no_rule_claims_is_still_converted() {
    let scratch = repository("other");
    scratch.write("notes/meeting.md", "# A meeting\n");
    assert!(
        targets(&scratch.path)
            .iter()
            .any(|target| target == "other/notes-meeting.pdf"),
        "a document with no category was dropped"
    );
}

#[test]
fn no_document_is_converted_twice() {
    let scratch = repository("once");
    scratch.write("notes/one.md", "# One\n");
    let found = targets(&scratch.path);
    let mut sorted = found.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), found.len(), "{found:?} holds a repeat");
}

#[test]
fn what_a_build_wrote_is_not_a_document() {
    let scratch = repository("skipped");
    scratch.write("target/pdf/handbook/01-vision.md", "# Not this\n");
    scratch.write("research/other-project/README.md", "# Not this either\n");
    let found = targets(&scratch.path);
    assert!(!found.iter().any(|target| target.contains("not-this")));
    assert!(!found.iter().any(|target| target.contains("target")));
    assert!(!found.iter().any(|target| target.contains("research")));
}

#[test]
fn the_title_of_a_document_is_its_first_heading() {
    let scratch = repository("titles");
    let sources = collect(&scratch.path).unwrap_or_default();
    let title = sources
        .iter()
        .find(|source| source.target.ends_with("readme.pdf"))
        .map(|source| source.title.clone());
    assert_eq!(title, Some("The Project".to_owned()));
}

#[test]
fn a_document_with_no_heading_is_called_by_its_file_name() {
    let scratch = repository("nameless");
    scratch.write("PLAIN.md", "Just text, no heading.\n");
    let sources = collect(&scratch.path).unwrap_or_default();
    let title = sources
        .iter()
        .find(|source| source.target.ends_with("plain.pdf"))
        .map(|source| source.title.clone());
    assert_eq!(title, Some("PLAIN".to_owned()));
}

#[test]
fn an_rfc_is_called_by_its_number_and_laid_out_as_one() {
    let scratch = repository("rfcs");
    let sources = collect(&scratch.path).unwrap_or_default();
    let rfc = sources
        .iter()
        .find(|source| source.target.ends_with("rfc791.pdf"));
    assert_eq!(
        rfc.map(|source| source.title.clone()),
        Some("RFC 791".to_owned())
    );
    assert_eq!(rfc.map(|source| source.kind), Some(Kind::Rfc));
}

#[test]
fn an_html_document_is_called_by_its_title_and_read_as_html() {
    let scratch = repository("standards");
    let sources = collect(&scratch.path).unwrap_or_default();
    let spec = sources
        .iter()
        .find(|source| source.target.ends_with("spec.pdf"));
    assert_eq!(
        spec.map(|source| source.title.clone()),
        Some("A Standard".to_owned())
    );
    assert_eq!(spec.map(|source| source.kind), Some(Kind::Html));
}

#[test]
fn an_html_document_with_no_title_is_called_by_its_file_name() {
    let scratch = repository("untitled");
    scratch.write("docs/ecma/loose.html", "<body><p>text</p></body>");
    let sources = collect(&scratch.path).unwrap_or_default();
    let spec = sources
        .iter()
        .find(|source| source.target.ends_with("loose.pdf"));
    assert_eq!(
        spec.map(|source| source.title.clone()),
        Some("loose".to_owned())
    );
}

#[test]
fn every_source_says_where_it_came_from() {
    let scratch = repository("origins");
    for source in collect(&scratch.path).unwrap_or_default() {
        assert!(
            !source.origin.starts_with('/'),
            "{} is not a path inside the repository",
            source.origin
        );
        assert!(scratch.path.join(&source.origin).is_file());
    }
}

#[test]
fn a_repository_with_nothing_in_it_yields_nothing() {
    let scratch = Scratch::new("empty");
    assert!(targets(&scratch.path).is_empty());
}
