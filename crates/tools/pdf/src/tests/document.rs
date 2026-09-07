// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The whole document, and the file it becomes.

use crate::document::Document;
use crate::font::Font;
use crate::page::{Link, LinkTarget, Page};
use crate::units::{Color, PageSize, pt};

fn sample() -> Document {
    let mut document = Document::new("A title");
    let mut first = Page::new(PageSize::A4);
    first.text(Font::Bold, pt(18), pt(72), pt(700), Color::BLACK, "Heading");
    first.link(Link {
        x: pt(72),
        y: pt(690),
        width: pt(100),
        height: pt(12),
        target: LinkTarget::Uri("https://example.invalid/a".to_owned()),
    });
    let index = document.push(first);
    document.outline(0, "Heading", index, pt(720));
    let mut second = Page::new(PageSize::A4);
    second.text(Font::Regular, pt(10), pt(72), pt(700), Color::BLACK, "Body");
    second.link(Link {
        x: pt(72),
        y: pt(690),
        width: pt(100),
        height: pt(12),
        target: LinkTarget::Page {
            index: 0,
            top: pt(720),
        },
    });
    let index = document.push(second);
    document.outline(1, "Sub", index, pt(720));
    document
}

fn text(document: &Document) -> String {
    String::from_utf8_lossy(&document.finish()).into_owned()
}

#[test]
fn a_new_document_holds_no_page() {
    let document = Document::new("empty");
    assert!(document.is_empty());
    assert_eq!(document.len(), 0);
    assert_eq!(document.next_index(), 0);
}

#[test]
fn pages_are_numbered_in_the_order_they_arrive() {
    let mut document = Document::new("counted");
    assert_eq!(document.push(Page::new(PageSize::A4)), 0);
    assert_eq!(document.next_index(), 1);
    assert_eq!(document.push(Page::new(PageSize::A4)), 1);
    assert_eq!(document.len(), 2);
}

#[test]
fn a_file_starts_with_the_header_and_ends_with_the_marker() {
    let bytes = sample().finish();
    assert!(bytes.starts_with(b"%PDF-1.7\n%"));
    assert!(bytes.ends_with(b"%%EOF\n"));
}

#[test]
fn the_page_tree_names_every_page() {
    let content = text(&sample());
    assert!(content.contains("/Type /Pages /Count 2"));
    assert!(content.contains("/Kids [ 11 0 R 13 0 R ]"));
}

#[test]
fn every_stream_declares_the_length_it_has() {
    let bytes = sample().finish();
    let content = String::from_utf8_lossy(&bytes).into_owned();
    let mut streams = 0usize;
    for piece in content.split("/Length ").skip(1) {
        let Some((declared, rest)) = piece.split_once(" >>\nstream\n") else {
            continue;
        };
        let declared: usize = declared.parse().unwrap_or_default();
        // The length counts the data and not the end of line that
        // separates it from the keyword, which is the form the format
        // asks for.
        let actual = rest.find("\nendstream").unwrap_or_default();
        assert_eq!(
            declared, actual,
            "a stream declares a length it does not have"
        );
        streams = streams.saturating_add(1);
    }
    assert_eq!(streams, 2, "one stream per page");
}

#[test]
fn the_cross_reference_table_points_at_every_object() {
    // The offsets are byte offsets, and the header carries four bytes
    // that are not text, so this test works on the bytes themselves. Read
    // as a string, every one of those four would grow to three and every
    // offset in the table would be wrong by eight.
    let bytes = sample().finish();
    let table = find(&bytes, b"\nxref\n").unwrap_or_default();
    let text = String::from_utf8_lossy(bytes.get(table..).unwrap_or_default()).into_owned();
    // The empty piece before the newline, `xref`, the range, and the
    // free entry: the objects start on the fifth line.
    let lines = text.lines().skip(4);
    let total: usize = text
        .lines()
        .nth(2)
        .and_then(|header| header.split_whitespace().nth(1))
        .and_then(|count| count.parse().ok())
        .unwrap_or_default();
    // Four fixed objects, six fonts, two per page, one per outline
    // entry, and the free entry the table always starts with.
    assert_eq!(total, 17);
    for (number, line) in lines.take(total.saturating_sub(1)).enumerate() {
        let number = number.saturating_add(1);
        let offset: usize = line
            .split_whitespace()
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or_default();
        assert!(offset > 0, "object {number} has no offset");
        let head = bytes.get(offset..).unwrap_or_default();
        assert!(
            head.starts_with(format!("{number} 0 obj").as_bytes()),
            "object {number} is not where the table says"
        );
    }
}

/// The offset of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[test]
fn the_outline_is_a_tree_the_catalogue_points_at() {
    let content = text(&sample());
    assert!(content.contains("/Outlines 4 0 R"));
    assert!(content.contains("/PageMode /UseOutlines"));
    assert!(content.contains("/Type /Outlines /First 15 0 R /Last 15 0 R /Count 2"));
    assert!(content.contains("/Title (Sub) /Parent 15 0 R"));
}

#[test]
fn a_document_with_no_heading_has_no_outline() {
    let mut document = Document::new("plain");
    document.push(Page::new(PageSize::A4));
    let content = text(&document);
    assert!(!content.contains("/Outlines 4 0 R"));
    assert!(content.contains("/Type /Outlines >>"));
}

#[test]
fn a_heading_that_skips_a_level_is_lifted_to_the_one_below_its_predecessor() {
    let mut document = Document::new("skipping");
    document.push(Page::new(PageSize::A4));
    document.outline(0, "a", 0, pt(700));
    document.outline(3, "b", 0, pt(600));
    let content = text(&document);
    assert!(content.contains("/Title (b) /Parent 13 0 R"));
}

#[test]
fn both_kinds_of_link_reach_the_page() {
    let content = text(&sample());
    assert!(content.contains("/A << /S /URI /URI (https://example.invalid/a) >>"));
    assert!(content.contains("/Dest [11 0 R /XYZ null 720 null]"));
}

#[test]
fn the_pages_are_numbered_from_after_the_fonts() {
    assert_eq!(
        usize::try_from(crate::document::FONTS).unwrap_or_default(),
        Font::ALL.len(),
        "a font was added without moving the pages after it"
    );
}

#[test]
fn no_two_objects_carry_the_same_number() {
    let bytes = sample().finish();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut numbers: Vec<&str> = text
        .match_indices(" 0 obj")
        .filter_map(|(at, _)| text.get(..at))
        .filter_map(|before| before.rsplit(['\n', ' ']).next())
        .collect();
    let found = numbers.len();
    numbers.sort_unstable();
    numbers.dedup();
    assert_eq!(
        numbers.len(),
        found,
        "two objects share a number: {numbers:?}"
    );
}

#[test]
fn every_font_of_the_crate_has_a_dictionary_of_its_own() {
    let text = text(&sample());
    for font in Font::ALL {
        assert!(
            text.contains(&format!("/BaseFont /{}", font.base_name())),
            "{} has no dictionary",
            font.base_name()
        );
        assert!(
            text.contains(&format!("/{} ", font.resource())),
            "{} is in no resource dictionary",
            font.resource()
        );
    }
}

#[test]
fn two_runs_write_the_same_bytes() {
    assert_eq!(sample().finish(), sample().finish());
}

#[test]
fn a_page_with_no_link_carries_no_annotation_array() {
    let mut document = Document::new("plain");
    let mut page = Page::new(PageSize::A4);
    page.text(Font::Regular, pt(10), 0, pt(700), Color::BLACK, "text");
    document.push(page);
    assert!(!text(&document).contains("/Annots"));
}

#[test]
fn the_five_fonts_are_declared_on_every_page() {
    let content = text(&sample());
    assert_eq!(content.matches("/F1 5 0 R").count(), 2);
    assert!(content.contains("/BaseFont /Helvetica-Oblique /Encoding /WinAnsiEncoding"));
    assert!(content.contains("/BaseFont /Courier-Bold"));
}

#[test]
fn the_title_of_a_document_reaches_the_information_dictionary() {
    let mut document = Document::new("A (parenthesised) title");
    document.push(Page::new(PageSize::A4));
    assert!(text(&document).contains("/Title (A \\(parenthesised\\) title)"));
}

/// A document with enough on its page that deflating it pays.
fn wordy() -> Document {
    let mut document = Document::new("A wordy document");
    let mut page = Page::new(PageSize::A4);
    for line in 0..40 {
        page.text(
            Font::Regular,
            pt(10),
            pt(64),
            pt(700).saturating_sub(pt(line).saturating_mul(14)),
            Color::BLACK,
            "a line of prose that says very much the same as the line above it",
        );
    }
    document.push(page);
    document.outline(0, "Heading", 0, pt(700));
    document
}

#[test]
fn a_compressed_document_says_so_and_reads_back() {
    let mut document = wordy();
    document.compress(true);
    let bytes = document.finish();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert!(text.contains("/Filter /FlateDecode"), "no filter was named");
    // The stream a viewer would read: the wrapper of RFC 1950 around the
    // format of RFC 1951, which is what `FlateDecode` means.
    let opening = "/Filter /FlateDecode >>\nstream\n";
    let start = find(&bytes, opening.as_bytes()).unwrap_or_default() + opening.len();
    let end = find(&bytes, b"\nendstream").unwrap_or_default();
    let stream = bytes.get(start..end).unwrap_or_default();
    let mut out = vec![0u8; 64 * 1024];
    let read = audhsos_deflate::decompress_zlib(stream, &mut out).expect("the stream reads back");
    let content = String::from_utf8_lossy(out.get(..read).unwrap_or_default()).into_owned();
    assert!(content.contains("(a line of prose"), "{content}");
}

#[test]
fn a_document_that_is_not_compressed_carries_no_filter() {
    assert!(!text(&wordy()).contains("/Filter"));
}

#[test]
fn compressing_makes_the_file_smaller_and_changes_nothing_else() {
    let plain = wordy().finish();
    let mut document = wordy();
    document.compress(true);
    let packed = document.finish();
    assert!(
        packed.len() < plain.len() / 2,
        "{} bytes became {}",
        plain.len(),
        packed.len()
    );
    assert!(packed.starts_with(b"%PDF-1.7"));
    assert!(packed.ends_with(b"%%EOF\n"));
}

#[test]
fn a_page_with_nothing_on_it_is_not_compressed() {
    let mut document = Document::new("empty");
    document.compress(true);
    document.push(Page::new(PageSize::A4));
    assert!(!text(&document).contains("/Filter"));
}
