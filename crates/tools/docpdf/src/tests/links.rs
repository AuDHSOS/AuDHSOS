// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Links between documents.

use std::path::PathBuf;

use crate::links::Links;
use crate::sources::{Kind, Source};

fn source(origin: &str, target: &str) -> Source {
    Source {
        path: PathBuf::from(origin),
        target: PathBuf::from(target),
        kind: Kind::Markdown,
        title: origin.to_owned(),
        origin: origin.to_owned(),
        size: 0,
    }
}

fn table() -> (Links, Vec<Source>) {
    let sources = vec![
        source("README.md", "project/readme.pdf"),
        source(
            "docs/06-testing-strategy.md",
            "handbook/06-testing-strategy.pdf",
        ),
        source("docs/12-parallel-work.md", "handbook/12-parallel-work.pdf"),
        source("crates/net/ip/README.md", "crates/net-ip.pdf"),
    ];
    (Links::new(&sources), sources)
}

#[test]
fn a_link_between_two_chapters_becomes_a_link_between_two_files() {
    let (links, sources) = table();
    let from = sources.get(1).cloned().unwrap_or_else(|| source("a", "a"));
    assert_eq!(
        links.resolve(&from, "12-parallel-work.md"),
        "12-parallel-work.pdf"
    );
}

#[test]
fn a_link_that_climbs_out_of_a_crate_finds_the_handbook() {
    let (links, sources) = table();
    let from = sources.get(3).cloned().unwrap_or_else(|| source("a", "a"));
    assert_eq!(
        links.resolve(&from, "../../../docs/06-testing-strategy.md"),
        "../handbook/06-testing-strategy.pdf"
    );
}

#[test]
fn a_link_from_the_root_reaches_a_crate() {
    let (links, sources) = table();
    let from = sources.first().cloned().unwrap_or_else(|| source("a", "a"));
    assert_eq!(
        links.resolve(&from, "crates/net/ip/README.md"),
        "../crates/net-ip.pdf"
    );
}

#[test]
fn an_anchor_on_a_converted_document_still_reaches_the_document() {
    let (links, sources) = table();
    let from = sources.get(1).cloned().unwrap_or_else(|| source("a", "a"));
    assert_eq!(
        links.resolve(&from, "12-parallel-work.md#phases"),
        "12-parallel-work.pdf"
    );
}

#[test]
fn an_address_on_the_web_is_left_alone() {
    let (links, sources) = table();
    let from = sources.first().cloned().unwrap_or_else(|| source("a", "a"));
    for href in [
        "https://example.invalid/a",
        "http://example.invalid",
        "mailto:someone@example.invalid",
        "#a-heading",
    ] {
        assert_eq!(links.resolve(&from, href), href);
    }
}

#[test]
fn a_link_to_something_that_was_not_converted_is_left_alone() {
    let (links, sources) = table();
    let from = sources.first().cloned().unwrap_or_else(|| source("a", "a"));
    for href in ["src/lib.rs", "docs/", "../nowhere.md", "../../../../up.md"] {
        assert_eq!(links.resolve(&from, href), href);
    }
}

#[test]
fn a_document_can_link_to_itself() {
    let (links, sources) = table();
    let from = sources.get(1).cloned().unwrap_or_else(|| source("a", "a"));
    assert_eq!(
        links.resolve(&from, "06-testing-strategy.md"),
        "06-testing-strategy.pdf"
    );
}
