// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Whole documents, from source text to written bytes.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::layout::Layout;
use crate::links::Links;
use crate::rfc;
use crate::sources::{Kind, Source};

fn source(kind: Kind) -> Source {
    Source {
        path: PathBuf::from("docs/example.md"),
        target: PathBuf::from("handbook/example.pdf"),
        kind,
        title: "An Example".to_owned(),
        origin: "docs/example.md".to_owned(),
        size: 0,
    }
}

fn markdown(text: &str) -> (Vec<u8>, usize) {
    let source = source(Kind::Markdown);
    let links = Links::new(std::slice::from_ref(&source));
    let blocks = doc_markdown::parse(text);
    let mut layout = Layout::new(&source, &links);
    layout.document(&blocks);
    let pages = layout.pages();
    (layout.finish(), pages)
}

fn text_of(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_document_becomes_a_pdf() {
    let (bytes, pages) = markdown("# An Example\n\nOne paragraph.\n");
    assert!(bytes.starts_with(b"%PDF-1.7"));
    assert!(bytes.ends_with(b"%%EOF\n"));
    assert_eq!(pages, 1);
}

#[test]
fn an_empty_document_is_still_a_title_page() {
    let (bytes, pages) = markdown("");
    assert_eq!(pages, 1);
    assert!(text_of(&bytes).contains("(An Example) Tj"));
}

#[test]
fn a_long_document_runs_onto_further_pages() {
    let mut text = String::from("# An Example\n\n");
    for index in 0..300 {
        let _ = writeln!(text, "Paragraph number {index} of the example document.\n");
    }
    let (_, pages) = markdown(&text);
    assert!(pages > 5, "300 paragraphs came out on {pages} page(s)");
}

#[test]
fn a_heading_becomes_an_outline_entry() {
    let (bytes, _) = markdown("# An Example\n\n## A section\n\ntext\n\n### Deeper\n\ntext\n");
    let content = text_of(&bytes);
    assert!(content.contains("/Title (A section)"));
    assert!(content.contains("/Title (Deeper)"));
    assert!(content.contains("/PageMode /UseOutlines"));
}

#[test]
fn the_title_is_not_set_twice() {
    let (bytes, _) = markdown("# An Example\n\ntext\n");
    let content = text_of(&bytes);
    assert_eq!(content.matches("(An Example) Tj").count(), 1);
}

#[test]
fn a_link_becomes_an_annotation() {
    let (bytes, _) = markdown("# An Example\n\nSee [the site](https://example.invalid/a).\n");
    assert!(text_of(&bytes).contains("/URI (https://example.invalid/a)"));
}

#[test]
fn a_code_block_keeps_its_lines_and_gets_a_ground() {
    let (bytes, _) = markdown("# An Example\n\n```\nfn main() {}\n```\n");
    let content = text_of(&bytes);
    assert!(content.contains("(fn main\\(\\) {}) Tj"));
    assert!(
        content.contains(" re f"),
        "the ground of the block is missing"
    );
}

#[test]
fn a_table_reaches_the_page() {
    let (bytes, _) =
        markdown("# An Example\n\n| Object | Purpose |\n| --- | --- |\n| Reply | answering |\n");
    let content = text_of(&bytes);
    assert!(content.contains("(Object) Tj"));
    assert!(content.contains("(answering) Tj"));
}

#[test]
fn a_list_gets_its_markers() {
    let (bytes, _) = markdown("# An Example\n\n- one\n- two\n\n1. first\n2. second\n");
    let content = text_of(&bytes);
    assert_eq!(content.matches("(\\225) Tj").count(), 2, "two bullets");
    assert!(content.contains("(1.) Tj"));
    assert!(content.contains("(2.) Tj"));
}

#[test]
fn two_runs_over_the_same_document_write_the_same_bytes() {
    let text = "# An Example\n\n## A section\n\nSome text with `code` and [a link](x.md).\n";
    assert_eq!(markdown(text).0, markdown(text).0);
}

#[test]
fn an_rfc_becomes_one_page_per_sheet() {
    let mut text = String::from("Network Working Group\n\n1. Introduction\n\nText.\n");
    text.push('\u{c}');
    text.push_str("Second sheet.\n");
    text.push('\u{c}');
    text.push_str("Third sheet.\n");
    let (bytes, pages) = rfc::render(&source(Kind::Rfc), &text);
    assert_eq!(pages, 3);
    assert!(bytes.starts_with(b"%PDF-1.7"));
}

#[test]
fn the_sections_of_an_rfc_become_its_outline() {
    let text = "1. Introduction\n\nText.\n\n2.1. Details\n\nMore.\n";
    let (bytes, _) = rfc::render(&source(Kind::Rfc), text);
    let content = text_of(&bytes);
    assert!(content.contains("/Title (1. Introduction)"));
    assert!(content.contains("/Title (2.1. Details)"));
}

#[test]
fn a_table_of_contents_line_is_not_a_section() {
    let text = "1. Introduction .......................... 3\n\n1. Introduction\n\nText.\n";
    let (bytes, _) = rfc::render(&source(Kind::Rfc), text);
    let content = text_of(&bytes);
    assert_eq!(content.matches("/Title (1. Introduction)").count(), 1);
}

#[test]
fn an_rfc_keeps_every_column_where_it_was() {
    let diagram =
        "    0                   1\n    0 1 2 3 4 5 6 7 8 9 0 1\n   +-+-+-+-+-+-+-+-+-+-+-+-+\n";
    let (bytes, _) = rfc::render(&source(Kind::Rfc), diagram);
    let content = text_of(&bytes);
    assert!(content.contains("(   +-+-+-+-+-+-+-+-+-+-+-+-+) Tj"));
}
