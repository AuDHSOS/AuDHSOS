// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One page and the operators it collects.

use crate::font::Font;
use crate::page::{Fill, Link, LinkTarget, Page, Segment, Stroke};
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
    assert!(content.contains("BT\n"), "{content}");
    assert!(content.contains("/F1 12 Tf"), "{content}");
    assert!(content.contains("1 0 0 1 72 700 Tm"), "{content}");
    assert!(content.contains("(hello) Tj"), "{content}");
}

#[test]
fn a_page_of_text_is_one_text_object() {
    let mut page = Page::new(PageSize::A4);
    for line in 0..4 {
        page.text(
            Font::Regular,
            pt(10),
            pt(64),
            pt(700).saturating_sub(pt(line * 14)),
            Color::BLACK,
            "a line of prose",
        );
    }
    page.end_text();
    let content = stream(&page);
    assert_eq!(content.matches("BT\n").count(), 1, "{content}");
    assert_eq!(content.matches("ET\n").count(), 1, "{content}");
    assert_eq!(content.matches("Tf\n").count(), 1, "{content}");
    assert_eq!(content.matches("Tm\n").count(), 1, "{content}");
    // The distance between the lines is said once, and every line after
    // the first is one string and the operator that means `next line'.
    assert_eq!(content.matches("14 TL\n").count(), 1, "{content}");
    assert_eq!(content.matches(")'\n").count(), 3, "{content}");
    assert_eq!(content.matches("Td\n").count(), 0, "{content}");
}

#[test]
fn a_run_that_begins_where_the_last_one_ended_says_nothing_about_its_place() {
    let mut page = Page::new(PageSize::A4);
    let width = Font::Regular.width_of_str("one ", pt(10));
    page.text(Font::Regular, pt(10), pt(64), pt(700), Color::BLACK, "one ");
    page.text(
        Font::Italic,
        pt(10),
        pt(64).saturating_add(width),
        pt(700),
        Color::BLACK,
        "two",
    );
    let content = stream(&page);
    assert_eq!(content.matches("Tm\n").count(), 1, "{content}");
    assert_eq!(content.matches("Td\n").count(), 0, "{content}");
    assert_eq!(content.matches(") Tj\n").count(), 2, "{content}");
}

#[test]
fn a_run_a_hair_away_from_the_last_one_still_says_where_it_is() {
    let mut page = Page::new(PageSize::A4);
    let width = Font::Regular.width_of_str("one ", pt(10));
    page.text(Font::Regular, pt(10), pt(64), pt(700), Color::BLACK, "one ");
    page.text(
        Font::Regular,
        pt(10),
        pt(64).saturating_add(width).saturating_add(1),
        pt(700),
        Color::BLACK,
        "two",
    );
    assert_eq!(stream(&page).matches("Td\n").count(), 1);
}

#[test]
fn what_follows_the_symbol_font_says_where_it_is() {
    // The widths of Symbol are a table this crate filled in by hand. A run
    // after one of them names its place rather than trusting that table.
    let mut page = Page::new(PageSize::A4);
    page.text(
        Font::Regular,
        pt(10),
        pt(64),
        pt(700),
        Color::BLACK,
        "x \u{221E}",
    );
    page.text(Font::Regular, pt(10), pt(200), pt(700), Color::BLACK, "y");
    let content = stream(&page);
    assert!(content.contains("/F6 10 Tf"), "{content}");
    assert_eq!(content.matches("Td\n").count(), 1, "{content}");
}

#[test]
fn a_face_is_named_again_only_when_it_changes() {
    let mut page = Page::new(PageSize::A4);
    page.text(Font::Regular, pt(10), 0, pt(700), Color::BLACK, "one");
    page.text(Font::Italic, pt(10), pt(50), pt(700), Color::BLACK, "two");
    page.text(Font::Italic, pt(10), pt(90), pt(700), Color::BLACK, "three");
    let content = stream(&page);
    assert_eq!(content.matches("Tf\n").count(), 2, "{content}");
}

#[test]
fn anything_that_is_not_text_takes_the_pen_back() {
    let mut page = Page::new(PageSize::A4);
    page.text(Font::Regular, pt(10), 0, pt(700), Color::BLACK, "one");
    page.rule(0, pt(690), pt(100), 300, Color::BLACK);
    page.text(Font::Regular, pt(10), 0, pt(680), Color::BLACK, "two");
    page.path(
        &[Segment::Move { x: 0, y: 0 }, Segment::Line { x: 10, y: 0 }],
        None,
        Some(&Stroke {
            color: Color::BLACK,
            width: 100,
            dashes: Vec::new(),
        }),
    );
    page.end_text();
    let content = stream(&page);
    assert_eq!(content.matches("BT\n").count(), 2, "{content}");
    assert_eq!(content.matches("ET\n").count(), 2, "{content}");
    // The second text object begins a line of its own, because the
    // matrices are the identity again inside it.
    assert_eq!(content.matches("Tm\n").count(), 2, "{content}");
}

#[test]
fn closing_a_page_that_set_no_text_writes_nothing() {
    let mut page = Page::new(PageSize::A4);
    page.end_text();
    assert!(page.is_empty());
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
    assert_eq!(stream(&page).matches(" g\n").count(), 1);
    page.text(Font::Regular, pt(9), 0, 0, Color::gray(128), "x");
    assert_eq!(stream(&page).matches(" g\n").count(), 2);
}

#[test]
fn a_grey_is_written_with_the_operator_that_takes_one_number() {
    let mut page = Page::new(PageSize::A4);
    page.text(Font::Regular, pt(9), 0, 0, Color::gray(26), "x");
    page.text(
        Font::Regular,
        pt(9),
        0,
        pt(20),
        Color::rgb(20, 70, 140),
        "x",
    );
    let content = stream(&page);
    assert!(content.contains("0.101 g\n"), "{content}");
    assert!(content.contains("0.078 0.274 0.549 rg\n"), "{content}");
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

#[test]
fn a_path_writes_the_operators_of_its_segments() {
    let mut page = Page::new(PageSize::A4);
    page.path(
        &[
            Segment::Move { x: pt(1), y: pt(2) },
            Segment::Line { x: pt(3), y: pt(4) },
            Segment::Curve {
                x1: pt(5),
                y1: pt(6),
                x2: pt(7),
                y2: pt(8),
                x: pt(9),
                y: pt(10),
            },
            Segment::Close,
        ],
        Some(Fill {
            color: Color::BLACK,
            even_odd: false,
        }),
        None,
    );
    let stream = stream(&page);
    assert!(stream.contains("1 2 m"), "{stream}");
    assert!(stream.contains("3 4 l"), "{stream}");
    assert!(stream.contains("5 6 7 8 9 10 c"), "{stream}");
    assert!(stream.contains("h\n"), "{stream}");
    assert!(stream.contains("f\n"), "{stream}");
}

#[test]
fn a_path_is_painted_the_way_it_is_asked_for() {
    let ground = [
        (Some(false), false, "f\n"),
        (Some(true), false, "f*\n"),
        (Some(false), true, "B\n"),
        (Some(true), true, "B*\n"),
        (None, true, "S\n"),
    ];
    for (fill, stroked, operator) in ground {
        let mut page = Page::new(PageSize::A4);
        page.path(
            &[Segment::Move { x: 0, y: 0 }, Segment::Line { x: 100, y: 0 }],
            fill.map(|even_odd| Fill {
                color: Color::BLACK,
                even_odd,
            }),
            stroked
                .then(|| Stroke {
                    color: Color::BLACK,
                    width: pt(1),
                    dashes: Vec::new(),
                })
                .as_ref(),
        );
        assert!(
            stream(&page).ends_with(operator),
            "{operator} is missing from {}",
            stream(&page)
        );
    }
}

#[test]
fn a_stroke_carries_its_width_its_colour_and_its_dashes() {
    let mut page = Page::new(PageSize::A4);
    page.path(
        &[Segment::Move { x: 0, y: 0 }, Segment::Line { x: 100, y: 0 }],
        None,
        Some(&Stroke {
            color: Color::rgb(255, 0, 0),
            width: 1500,
            dashes: vec![pt(2), pt(1)],
        }),
    );
    let stream = stream(&page);
    assert!(stream.contains("1.5 w"), "{stream}");
    assert!(stream.contains("[2 1] 0 d"), "{stream}");
    assert!(stream.contains("1 0 0 RG"), "{stream}");
}

#[test]
fn a_path_that_is_neither_filled_nor_stroked_is_not_drawn() {
    let mut page = Page::new(PageSize::A4);
    page.path(&[Segment::Move { x: 0, y: 0 }], None, None);
    assert!(page.is_empty());
    page.path(
        &[],
        Some(Fill {
            color: Color::BLACK,
            even_odd: false,
        }),
        None,
    );
    assert!(page.is_empty());
}

#[test]
fn a_colour_is_set_once_for_as_long_as_it_stands() {
    let mut page = Page::new(PageSize::A4);
    let stroke = Stroke {
        color: Color::BLACK,
        width: pt(1),
        dashes: Vec::new(),
    };
    let line = [Segment::Move { x: 0, y: 0 }, Segment::Line { x: 10, y: 0 }];
    page.path(&line, None, Some(&stroke));
    page.path(&line, None, Some(&stroke));
    assert_eq!(stream(&page).matches("0 G").count(), 1);
}
