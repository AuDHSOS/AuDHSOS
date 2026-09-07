// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What runs inside a block.

use crate::inline::{Inline, parse, plain};

fn text(source: &str) -> String {
    plain(&parse(source))
}

#[test]
fn plain_text_stays_one_run() {
    assert_eq!(
        parse("just words"),
        vec![Inline::Text("just words".to_owned())]
    );
}

#[test]
fn strong_and_emphasis_are_told_apart() {
    assert_eq!(
        parse("**bold**"),
        vec![Inline::Strong(vec![Inline::Text("bold".to_owned())])]
    );
    assert_eq!(
        parse("*thin*"),
        vec![Inline::Emphasis(vec![Inline::Text("thin".to_owned())])]
    );
}

#[test]
fn an_underscore_inside_a_word_is_a_character_and_not_a_mark() {
    assert_eq!(
        parse("saturating_add and check_crate_roots"),
        vec![Inline::Text(
            "saturating_add and check_crate_roots".to_owned()
        )]
    );
    assert_eq!(text("a _real_ mark"), "a real mark");
    assert!(
        parse("a _real_ mark")
            .iter()
            .any(|item| matches!(item, Inline::Emphasis(_)))
    );
}

#[test]
fn a_mark_with_a_space_after_it_opens_nothing() {
    assert_eq!(
        parse("2 * 3 * 4"),
        vec![Inline::Text("2 * 3 * 4".to_owned())]
    );
}

#[test]
fn a_code_span_keeps_what_is_inside_it() {
    assert_eq!(
        parse("`let x = *p;`"),
        vec![Inline::Code("let x = *p;".to_owned())]
    );
}

#[test]
fn a_code_span_may_contain_a_backtick() {
    assert_eq!(parse("``a ` b``"), vec![Inline::Code("a ` b".to_owned())]);
}

#[test]
fn a_code_span_drops_one_space_at_each_end() {
    assert_eq!(
        parse("`` `fence` ``"),
        vec![Inline::Code("`fence`".to_owned())]
    );
}

#[test]
fn an_unclosed_backtick_is_a_backtick() {
    assert_eq!(parse("a ` b"), vec![Inline::Text("a ` b".to_owned())]);
}

#[test]
fn a_link_keeps_its_text_and_its_target() {
    assert_eq!(
        parse("[the docs](docs/README.md)"),
        vec![Inline::Link {
            content: vec![Inline::Text("the docs".to_owned())],
            href: "docs/README.md".to_owned(),
        }]
    );
}

#[test]
fn a_link_title_is_not_part_of_the_target() {
    assert!(matches!(
        parse("[a](b.md \"a title\")").first(),
        Some(Inline::Link { href, .. }) if href == "b.md"
    ));
}

#[test]
fn a_bracket_that_opens_nothing_stays_a_bracket() {
    assert_eq!(
        parse("[not a link]"),
        vec![Inline::Text("[not a link]".to_owned())]
    );
}

#[test]
fn an_image_shows_what_it_describes() {
    assert_eq!(text("![a diagram](d.png)"), "[image: a diagram]");
}

#[test]
fn an_address_in_angle_brackets_is_a_link() {
    assert!(matches!(
        parse("<https://example.invalid/a>").first(),
        Some(Inline::Link { href, .. }) if href == "https://example.invalid/a"
    ));
}

#[test]
fn a_generic_in_angle_brackets_is_not_a_link() {
    assert_eq!(parse("<T>"), vec![Inline::Text("<T>".to_owned())]);
}

#[test]
fn a_backslash_hides_the_mark_behind_it() {
    assert_eq!(
        parse("\\*not emphasis\\*"),
        vec![Inline::Text("*not emphasis*".to_owned())]
    );
}

#[test]
fn a_soft_break_becomes_a_space_and_a_hard_break_stays_a_break() {
    assert_eq!(parse("one\ntwo"), vec![Inline::Text("one two".to_owned())]);
    assert_eq!(
        parse("one  \ntwo"),
        vec![
            Inline::Text("one".to_owned()),
            Inline::Break,
            Inline::Text("two".to_owned())
        ]
    );
}

#[test]
fn marks_nest() {
    assert_eq!(
        parse("**bold with `code`**"),
        vec![Inline::Strong(vec![
            Inline::Text("bold with ".to_owned()),
            Inline::Code("code".to_owned())
        ])]
    );
}

#[test]
fn the_plain_text_of_a_run_drops_every_mark() {
    assert_eq!(
        text("a **bold** `span` and [a link](x.md)"),
        "a bold span and a link"
    );
}

#[test]
fn a_backslash_before_something_that_is_not_a_mark_stays_a_backslash() {
    assert_eq!(parse("a \\b"), vec![Inline::Text("a \\b".to_owned())]);
    assert_eq!(
        parse("ends in \\"),
        vec![Inline::Text("ends in \\".to_owned())]
    );
}

#[test]
fn an_exclamation_mark_that_opens_no_image_is_an_exclamation_mark() {
    assert_eq!(parse("done!"), vec![Inline::Text("done!".to_owned())]);
    assert_eq!(
        parse("![unclosed"),
        vec![Inline::Text("![unclosed".to_owned())]
    );
}

#[test]
fn an_address_of_a_person_is_a_link_to_write_to_them() {
    assert!(matches!(
        parse("<someone@example.invalid>").first(),
        Some(Inline::Link { href, .. }) if href == "mailto:someone@example.invalid"
    ));
}

#[test]
fn an_empty_pair_of_angle_brackets_is_text() {
    assert_eq!(parse("<>"), vec![Inline::Text("<>".to_owned())]);
    assert_eq!(parse("a < b"), vec![Inline::Text("a < b".to_owned())]);
}

#[test]
fn a_target_may_hold_the_brackets_it_balances() {
    assert!(matches!(
        parse("[a](b(c)d.md)").first(),
        Some(Inline::Link { href, .. }) if href == "b(c)d.md"
    ));
    assert_eq!(
        parse("[a](unclosed"),
        vec![Inline::Text("[a](unclosed".to_owned())]
    );
}

#[test]
fn a_label_may_hold_the_brackets_it_balances() {
    assert!(matches!(
        parse("[a [b] c](d.md)").first(),
        Some(Inline::Link { href, .. }) if href == "d.md"
    ));
}

#[test]
fn an_image_with_no_description_shows_where_it_points() {
    assert_eq!(text("![](diagram.png)"), "[image: diagram.png]");
}

#[test]
fn a_code_span_of_only_spaces_keeps_them() {
    assert_eq!(parse("`  `"), vec![Inline::Code("  ".to_owned())]);
}

#[test]
fn a_code_span_may_run_over_a_line_break() {
    assert_eq!(
        parse("`one\ntwo`"),
        vec![Inline::Code("one two".to_owned())]
    );
}

#[test]
fn an_image_carries_the_file_it_is_in_and_what_it_shows() {
    assert_eq!(
        parse("![a diagram](d.png)"),
        vec![Inline::Image {
            source: "d.png".to_owned(),
            alt: "a diagram".to_owned(),
        }]
    );
}

#[test]
fn an_image_is_described_by_what_it_shows_or_by_where_it_is() {
    assert_eq!(
        crate::inline::described("d.png", "a diagram"),
        "[image: a diagram]"
    );
    assert_eq!(crate::inline::described("d.png", "  "), "[image: d.png]");
}
