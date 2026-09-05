// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One page and the operators it collects.

use crate::font::Font;
use crate::page::{Link, LinkTarget, Page};
use crate::units::{Color, PageSize, pt};

fn stream(page: &Page) -> String {
    String::from_utf8_lossy(page.content()).into_owned()
}

#[test]
fn a_new_page_has_drawn_nothing() {
    let page = Page::new(PageSize::A4);
    assert!(page.is_empty());
    assert_eq!(page.size(), PageSize::A4);
    assert!(page.links().is_empty());
}

#[test]
fn text_becomes_a_text_object() {
    let mut page = Page::new(PageSize::A4);
    page.text(
        Font::Regular,
        pt(12),
        pt(72),
        pt(700),
        Color::BLACK,
        "hello",
    );
    let content = stream(&page);
    assert!(content.contains("BT /F1 12 Tf"));
    assert!(content.contains("1 0 0 1 72 700 Tm"));
    assert!(content.contains("(hello) Tj ET"));
}

#[test]
fn empty_text_draws_nothing() {
    let mut page = Page::new(PageSize::A4);
    page.text(Font::Regular, pt(12), 0, 0, Color::BLACK, "");
    assert!(page.is_empty());
}

#[test]
fn the_delimiters_of_a_string_are_escaped() {
    let mut page = Page::new(PageSize::A4);
    page.text(Font::Mono, pt(9), 0, 0, Color::BLACK, "a(b)c\\d");
    assert!(stream(&page).contains("(a\\(b\\)c\\\\d) Tj"));
}

#[test]
fn a_high_byte_is_written_as_an_octal_escape() {
    let mut page = Page::new(PageSize::A4);
    page.text(Font::Regular, pt(9), 0, 0, Color::BLACK, "\u{2014}");
    assert!(stream(&page).contains("(\\227) Tj"));
}

#[test]
fn the_fill_colour_is_set_once_for_a_run_of_the_same_colour() {
    let mut page = Page::new(PageSize::A4);
    for line in 0..3 {
        page.text(Font::Regular, pt(9), 0, pt(line), Color::BLACK, "x");
    }
    assert_eq!(stream(&page).matches(" rg").count(), 1);
    page.text(Font::Regular, pt(9), 0, 0, Color::gray(128), "x");
    assert_eq!(stream(&page).matches(" rg").count(), 2);
}

#[test]
fn a_rectangle_with_no_area_is_not_drawn() {
    let mut page = Page::new(PageSize::A4);
    page.rect(0, 0, 0, pt(10), Color::BLACK);
    page.rect(0, 0, pt(10), -1, Color::BLACK);
    assert!(page.is_empty());
}

#[test]
fn a_rule_is_centred_on_its_line() {
    let mut page = Page::new(PageSize::A4);
    page.rule(pt(10), pt(100), pt(200), pt(2), Color::BLACK);
    assert!(stream(&page).contains("10 99 200 2 re f"));
}

#[test]
fn links_are_kept_in_the_order_they_arrive() {
    let mut page = Page::new(PageSize::A4);
    page.link(Link {
        x: 0,
        y: 0,
        width: pt(10),
        height: pt(10),
        target: LinkTarget::Uri("https://example.invalid".to_owned()),
    });
    page.link(Link {
        x: 0,
        y: pt(20),
        width: pt(10),
        height: pt(10),
        target: LinkTarget::Page {
            index: 3,
            top: pt(500),
        },
    });
    assert_eq!(page.links().len(), 2);
    assert!(matches!(
        page.links().get(1).map(|link| &link.target),
        Some(LinkTarget::Page { index: 3, .. })
    ));
}

#[test]
fn a_rule_of_no_thickness_is_not_drawn() {
    let mut page = Page::new(PageSize::LETTER);
    page.rule(0, pt(100), pt(200), 0, Color::BLACK);
    assert!(page.is_empty());
}

#[test]
fn text_of_only_spaces_is_still_text() {
    let mut page = Page::new(PageSize::A4);
    page.text(Font::Mono, pt(9), 0, 0, Color::BLACK, "   ");
    assert!(!page.is_empty());
}

#[test]
fn already_encoded_text_can_be_set_directly() {
    let mut page = Page::new(PageSize::A4);
    page.text_encoded(Font::Regular, pt(10), 0, 0, Color::BLACK, &[0x97]);
    assert!(stream(&page).contains("(\\227) Tj"));
    let mut empty = Page::new(PageSize::A4);
    empty.text_encoded(Font::Regular, pt(10), 0, 0, Color::BLACK, &[]);
    assert!(empty.is_empty());
}
